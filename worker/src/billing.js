import { verifyPolarSignature, parsePolarEvent } from "./polar.js";

// Polar webhook payloads expose the customer at either of these paths
// depending on event/expansion shape. Returns the Polar customer UUID
// (string) or null.
function resolvePolarCustomerId(eventData) {
  const d = eventData ?? {};
  return d.customer_id ?? d.customer?.id ?? null;
}

// Derive billing period length in months from period bounds (~30d ≈ 1 month).
// Used to scale credit grant: monthly = 1× allowance, yearly = 12× allowance.
function periodMonthsFromBounds(startIso, endIso) {
  if (!startIso || !endIso) return 1;
  const days = (Date.parse(endIso) - Date.parse(startIso)) / 86_400_000;
  if (!Number.isFinite(days) || days <= 0) return 1;
  return days >= 60 ? 12 : 1;
}

// Per spec §10b: webhook signature verification ALWAYS comes first.
// No DB write happens unless verification passes.
//
// Event handling is dispatched per `type`. Unknown types return 200
// (Polar's docs say to ack any 2xx to prevent retries; ignoring unknown
// types is safer than logging an error to operator alerts).

export async function handleBillingWebhook(request, env) {
  const webhookId = request.headers.get("webhook-id");
  if (!webhookId) return new Response("missing signature", { status: 401 });

  const rawBody = await request.text();
  const ok = await verifyPolarSignature(rawBody, request.headers, env);
  if (!ok) return new Response("bad signature", { status: 401 });

  const event = parsePolarEvent(rawBody);
  if (!event) return new Response("bad payload", { status: 400 });

  // Dedup on the delivery ID from the header (not payload event id).
  const existing = await env.CLAUGE_DB.prepare(
    "SELECT 1 FROM subscription_history WHERE polar_event_id = ?"
  )
    .bind(webhookId)
    .first();
  if (existing) return new Response("duplicate", { status: 200 });

  const userId = resolveUserId(event);
  if (userId === null) {
    return new Response("no user context", { status: 200 });
  }

  await dispatch(event, userId, env);
  await logEvent(event, webhookId, userId, rawBody, env);

  return new Response("ok", { status: 200 });
}

function resolveUserId(event) {
  // Polar webhooks may expose the external customer ID at any of these paths.
  // Check in order, take first non-null.
  const d = event.data ?? {};
  const candidates = [
    d.external_customer_id,
    d.customer?.external_id,
    d.customer_external_id,             // legacy field name
    d.order?.external_customer_id,
    d.order?.customer?.external_id,
    d.subscription?.external_customer_id,
    d.subscription?.customer?.external_id,
  ];
  for (const c of candidates) {
    if (c == null) continue;
    const n = Number(c);
    if (Number.isInteger(n) && n > 0) return n;
  }
  return null;
}

async function logEvent(event, webhookId, userId, rawBody, env) {
  await env.CLAUGE_DB.prepare(
    `INSERT OR IGNORE INTO subscription_history
       (user_id, event_type, polar_event_id, payload_json, occurred_at)
     VALUES (?, ?, ?, ?, ?)`
  )
    .bind(userId, event.type, webhookId, rawBody, event.created_at ?? new Date().toISOString())
    .run();
}

async function dispatch(event, userId, env) {
  // Per-event handlers live below — added in Tasks 7-8.
  // Unknown types are no-op (graceful, returns 200 from caller).
  switch (event.type) {
    case "subscription.created":
      return handleSubscriptionCreated(event, userId, env);
    case "subscription.updated":
    case "subscription.active":         // fires when sub becomes active; delegate to updated
    case "subscription.uncanceled":     // user toggled off cancel-at-period-end; delegate
    case "subscription.past_due":       // explicit past_due event; delegate
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
  const polarCustomerId = resolvePolarCustomerId(d);
  await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       plan = 'pro',
       subscription_status = ?,
       cancel_at_period_end = ?,
       current_period_start = ?,
       current_period_end = ?,
       polar_subscription_id = ?,
       polar_customer_id = COALESCE(?, polar_customer_id),
       credit_allowance_per_cycle = ?,
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
      polarCustomerId,
      await defaultCreditAllowance(env),
      userId
    )
    .run();
}

async function handleSubscriptionUpdated(event, userId, env) {
  const d = event.data;
  if (d.status === "past_due") {
    const polarCustomerId = resolvePolarCustomerId(d);
    await env.CLAUGE_DB.prepare(
      `UPDATE users SET
         subscription_status = 'past_due',
         past_due_started_at = COALESCE(past_due_started_at, CURRENT_TIMESTAMP),
         cancel_at_period_end = ?,
         current_period_end = COALESCE(?, current_period_end),
         polar_customer_id = COALESCE(?, polar_customer_id),
         updated_at = CURRENT_TIMESTAMP
       WHERE user_id = ?`
    )
      .bind(d.cancel_at_period_end ? 1 : 0, d.current_period_end ?? null, polarCustomerId, userId)
      .run();
    return;
  }

  if (d.status === "unpaid") {
    return revokeUser(userId, "unpaid", env);
  }

  const polarCustomerId = resolvePolarCustomerId(d);
  await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       subscription_status = ?,
       cancel_at_period_end = ?,
       current_period_start = COALESCE(?, current_period_start),
       current_period_end = COALESCE(?, current_period_end),
       polar_customer_id = COALESCE(?, polar_customer_id),
       past_due_started_at = CASE WHEN ? = 'active' THEN NULL ELSE past_due_started_at END,
       updated_at = CURRENT_TIMESTAMP
     WHERE user_id = ?`
  )
    .bind(
      d.status,
      d.cancel_at_period_end ? 1 : 0,
      d.current_period_start ?? null,
      d.current_period_end ?? null,
      polarCustomerId,
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
  // Period bounds live on the Subscription, surfaced via subscription.created/updated
  // (which fire BEFORE order.paid). The order.paid payload itself may not include
  // them, so we read the user row to get the authoritative bounds.
  const current = await env.CLAUGE_DB.prepare(
    "SELECT credit_allowance_per_cycle, current_period_start, current_period_end FROM users WHERE user_id = ?"
  )
    .bind(userId)
    .first();
  if (!current) return;
  const allowance = current.credit_allowance_per_cycle || (await defaultCreditAllowance(env));
  const months = periodMonthsFromBounds(current.current_period_start, current.current_period_end);
  const grantTotal = allowance * months;
  const polarCustomerId = resolvePolarCustomerId(d);
  await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       subscription_status = 'active',
       cancel_at_period_end = 0,
       past_due_started_at = NULL,
       current_period_start = COALESCE(?, current_period_start),
       current_period_end = COALESCE(?, current_period_end),
       polar_customer_id = COALESCE(?, polar_customer_id),
       credits_remaining = ?,
       updated_at = CURRENT_TIMESTAMP
     WHERE user_id = ?`
  )
    .bind(d.current_period_start ?? null, d.current_period_end ?? null, polarCustomerId, grantTotal, userId)
    .run();
}

async function handleOrderRefunded(event, userId, env) {
  await revokeUser(userId, "canceled", env);
}

function planToProductId(plan, env) {
  if (plan === "monthly") return env.POLAR_PRODUCT_MONTHLY;
  if (plan === "yearly") return env.POLAR_PRODUCT_YEARLY;
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

// Public, anonymous, edge-cacheable. Reads two small operator-managed tables.
// Returns a minimal contract — desktop app computes per-month math,
// slashed prices, and all UI formatting from these raw values.
//
// schema_version=1 lets clients gracefully handle future shape changes
// (rule: only add fields, never remove or rename).
export async function handleGetPricing(env) {
  const [pricingResult, discountResult] = await Promise.all([
    env.CLAUGE_DB.prepare(
      "SELECT plan_id AS id, price_usd FROM billing_pricing ORDER BY price_usd ASC"
    ).all(),
    env.CLAUGE_DB.prepare(
      "SELECT plan_id, percent, code FROM billing_discount"
    ).all(),
  ]);

  const discountsByPlan = new Map();
  for (const row of discountResult.results) {
    discountsByPlan.set(row.plan_id, { percent: row.percent, code: row.code });
  }

  const plans = pricingResult.results.map((p) => ({
    id: p.id,
    price_usd: p.price_usd,
    discount: discountsByPlan.get(p.id) ?? null,
  }));

  return new Response(
    JSON.stringify({ schema_version: 1, plans }),
    {
      status: 200,
      headers: {
        "content-type": "application/json",
        "cache-control": "public, max-age=300",
      },
    }
  );
}

export async function handleCreateCheckout(request, env, userId) {
  if (!userId) return new Response("unauthorized", { status: 401 });
  let body;
  try {
    body = await request.json();
  } catch {
    return new Response("bad json", { status: 400 });
  }
  const productId = planToProductId(body.plan, env);
  if (!productId) return new Response("invalid plan", { status: 400 });

  const userRow = await env.CLAUGE_DB.prepare(
    "SELECT primary_email, polar_customer_id FROM users WHERE user_id = ?"
  )
    .bind(userId)
    .first();

  const req = {
    products: [productId],
    external_customer_id: String(userId),
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
