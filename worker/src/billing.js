import { verifyPolarSignature, checkReplayWindow, parsePolarEvent } from "./polar.js";

// Per spec §10b: webhook signature verification ALWAYS comes first.
// No DB write happens unless verification passes.
//
// Event handling is dispatched per `type`. Unknown types return 200
// (Polar's docs say to ack any 2xx to prevent retries; ignoring unknown
// types is safer than logging an error to operator alerts).

export async function handleBillingWebhook(request, env) {
  const sigHex = request.headers.get("webhook-signature") || "";
  if (!sigHex) return new Response("missing signature", { status: 401 });

  const rawBody = await request.text();
  const ok = await verifyPolarSignature(rawBody, sigHex, env);
  if (!ok) return new Response("bad signature", { status: 401 });

  const event = parsePolarEvent(rawBody);
  if (!event) return new Response("bad payload", { status: 400 });

  if (!checkReplayWindow(event.created_at)) {
    return new Response("event too old", { status: 400 });
  }

  // Dedup check — UNIQUE constraint on polar_event_id handles the race,
  // but pre-checking avoids spurious INSERT-fail noise in logs.
  const existing = await env.CLAUGE_DB.prepare(
    "SELECT 1 FROM subscription_history WHERE polar_event_id = ?"
  )
    .bind(event.id)
    .first();
  if (existing) return new Response("duplicate", { status: 200 });

  const userId = resolveUserId(event);
  if (userId === null) {
    // Some events (organization.*) don't carry a user — skip silently.
    return new Response("no user context", { status: 200 });
  }

  await dispatch(event, userId, env);
  await logEvent(event, userId, rawBody, env);

  return new Response("ok", { status: 200 });
}

function resolveUserId(event) {
  // Checkout is configured to pass user_id as external_customer_id;
  // most event payloads expose it at data.customer.external_id.
  const d = event.data ?? {};
  const ext =
    d.customer?.external_id ??
    d.order?.customer?.external_id ??
    d.subscription?.customer?.external_id;
  if (!ext) return null;
  const n = Number(ext);
  return Number.isInteger(n) && n > 0 ? n : null;
}

async function logEvent(event, userId, rawBody, env) {
  await env.CLAUGE_DB.prepare(
    `INSERT OR IGNORE INTO subscription_history
       (user_id, event_type, polar_event_id, payload_json, occurred_at)
     VALUES (?, ?, ?, ?, ?)`
  )
    .bind(userId, event.type, event.id, rawBody, event.created_at)
    .run();
}

async function dispatch(event, userId, env) {
  // Per-event handlers live below — added in Tasks 7-8.
  // Unknown types are no-op (graceful, returns 200 from caller).
  switch (event.type) {
    case "subscription.created":
      return handleSubscriptionCreated(event, userId, env);
    case "subscription.updated":
      return handleSubscriptionUpdated(event, userId, env);
    case "subscription.canceled":
      return handleSubscriptionCanceled(event, userId, env);
    case "subscription.revoked":
      return handleSubscriptionRevoked(event, userId, env);
    case "order.created":
      return; // pending — no-op until order.paid
    case "order.paid":
      return handleOrderPaid(event, userId, env);
    case "order.refunded":
      return handleOrderRefunded(event, userId, env);
    case "customer.state_changed":
      return; // we derive everything from sub/order events
    default:
      return;
  }
}

// Per-cycle credit allowance lookup. Today stored per-user (set on
// subscription.created). Spec §13: tunable via D1, not hardcoded.
// For initial provisioning, we read from KV `pro:default_allowance`
// (operator sets), defaulting to 1000 if unset.

async function defaultCreditAllowance(env) {
  const raw = await env.CLAUGE_KV.get("pro:default_allowance");
  const n = raw ? Number(raw) : NaN;
  return Number.isInteger(n) && n > 0 ? n : 1000;
}

async function handleSubscriptionCreated(event, userId, env) {
  const d = event.data;
  const allowance = await defaultCreditAllowance(env);
  await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       plan = 'pro',
       subscription_status = ?,
       cancel_at_period_end = ?,
       current_period_start = ?,
       current_period_end = ?,
       polar_subscription_id = ?,
       credit_allowance_per_cycle = ?,
       credits_remaining = ?,
       past_due_started_at = NULL,
       updated_at = CURRENT_TIMESTAMP
     WHERE user_id = ?`
  )
    .bind(
      d.status === "trialing" ? "active" : d.status,
      d.cancel_at_period_end ? 1 : 0,
      d.current_period_start,
      d.current_period_end,
      d.id,
      allowance,
      allowance,
      userId
    )
    .run();
}

async function handleSubscriptionUpdated(event, userId, env) {
  const d = event.data;
  if (d.status === "past_due") {
    await env.CLAUGE_DB.prepare(
      `UPDATE users SET
         subscription_status = 'past_due',
         past_due_started_at = COALESCE(past_due_started_at, CURRENT_TIMESTAMP),
         cancel_at_period_end = ?,
         current_period_end = COALESCE(?, current_period_end),
         updated_at = CURRENT_TIMESTAMP
       WHERE user_id = ?`
    )
      .bind(d.cancel_at_period_end ? 1 : 0, d.current_period_end ?? null, userId)
      .run();
    return;
  }

  if (d.status === "unpaid") {
    return revokeUser(userId, "unpaid", env);
  }

  await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       subscription_status = ?,
       cancel_at_period_end = ?,
       current_period_start = COALESCE(?, current_period_start),
       current_period_end = COALESCE(?, current_period_end),
       past_due_started_at = CASE WHEN ? = 'active' THEN NULL ELSE past_due_started_at END,
       updated_at = CURRENT_TIMESTAMP
     WHERE user_id = ?`
  )
    .bind(
      d.status,
      d.cancel_at_period_end ? 1 : 0,
      d.current_period_start ?? null,
      d.current_period_end ?? null,
      d.status,
      userId
    )
    .run();
}

async function handleSubscriptionCanceled(event, userId, env) {
  const d = event.data;
  await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       cancel_at_period_end = 1,
       current_period_end = COALESCE(?, current_period_end),
       updated_at = CURRENT_TIMESTAMP
     WHERE user_id = ?`
  )
    .bind(d.current_period_end ?? null, userId)
    .run();
}

async function handleSubscriptionRevoked(event, userId, env) {
  await revokeUser(userId, "canceled", env);
}

async function revokeUser(userId, terminalStatus, env) {
  await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       plan = 'free',
       subscription_status = ?,
       cancel_at_period_end = 0,
       credits_remaining = 0,
       past_due_started_at = NULL,
       updated_at = CURRENT_TIMESTAMP
     WHERE user_id = ?`
  )
    .bind(terminalStatus, userId)
    .run();
}

async function handleOrderPaid(event, userId, env) {
  const d = event.data;
  // Idempotency: only reset credits if the order's period_start differs
  // from the user's currently-stored period_start. Avoids double-grant
  // when subscription.created already ran for the same period.
  const current = await env.CLAUGE_DB.prepare(
    "SELECT current_period_start, credit_allowance_per_cycle FROM users WHERE user_id = ?"
  )
    .bind(userId)
    .first();
  if (!current) return;

  const incomingPeriodStart = d.current_period_start ?? null;
  if (incomingPeriodStart && incomingPeriodStart === current.current_period_start) {
    return; // already provisioned for this cycle
  }

  const allowance = current.credit_allowance_per_cycle || (await defaultCreditAllowance(env));
  await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       subscription_status = 'active',
       cancel_at_period_end = 0,
       past_due_started_at = NULL,
       current_period_start = COALESCE(?, current_period_start),
       current_period_end = COALESCE(?, current_period_end),
       credits_remaining = ?,
       updated_at = CURRENT_TIMESTAMP
     WHERE user_id = ?`
  )
    .bind(incomingPeriodStart, d.current_period_end ?? null, allowance, userId)
    .run();
}

async function handleOrderRefunded(event, userId, env) {
  await revokeUser(userId, "canceled", env);
}

function planToPriceId(plan, env) {
  if (plan === "monthly") return env.POLAR_PRICE_MONTHLY;
  if (plan === "yearly") return env.POLAR_PRICE_YEARLY;
  return null;
}

export async function handleCreatePortal(env, userId) {
  if (!userId) return new Response("unauthorized", { status: 401 });
  const row = await env.CLAUGE_DB.prepare(
    "SELECT polar_customer_id FROM users WHERE user_id = ?"
  )
    .bind(userId)
    .first();
  if (!row?.polar_customer_id) return new Response("no customer", { status: 404 });

  const resp = await fetch("https://api.polar.sh/v1/customer-sessions/", {
    method: "POST",
    headers: {
      authorization: `Bearer ${env.POLAR_API_KEY}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({ customer_id: row.polar_customer_id }),
  });
  if (!resp.ok) {
    const text = await resp.text();
    return new Response("polar portal create failed: " + text.slice(0, 200), { status: 502 });
  }
  const data = await resp.json();
  return new Response(JSON.stringify({ url: data.customer_portal_url }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

export async function handleCreateCheckout(request, env, userId) {
  if (!userId) return new Response("unauthorized", { status: 401 });
  let body;
  try {
    body = await request.json();
  } catch {
    return new Response("bad json", { status: 400 });
  }
  const priceId = planToPriceId(body.plan, env);
  if (!priceId) return new Response("invalid plan", { status: 400 });

  const userRow = await env.CLAUGE_DB.prepare(
    "SELECT primary_email, polar_customer_id FROM users WHERE user_id = ?"
  )
    .bind(userId)
    .first();

  const req = {
    product_price_id: priceId,
    customer_external_id: String(userId),
    customer_email: userRow?.primary_email ?? undefined,
    success_url: "https://clauge.in/upgrade-success?ref=" + encodeURIComponent(String(userId)),
  };
  const resp = await fetch("https://api.polar.sh/v1/checkouts/", {
    method: "POST",
    headers: {
      authorization: `Bearer ${env.POLAR_API_KEY}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(req),
  });
  if (!resp.ok) {
    const text = await resp.text();
    return new Response("polar checkout create failed: " + text.slice(0, 200), { status: 502 });
  }
  const data = await resp.json();
  return new Response(JSON.stringify({ url: data.url }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}
