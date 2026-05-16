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

// Stubs — implemented in Tasks 7 & 8.
async function handleSubscriptionCreated(event, userId, env) {}
async function handleSubscriptionUpdated(event, userId, env) {}
async function handleSubscriptionCanceled(event, userId, env) {}
async function handleSubscriptionRevoked(event, userId, env) {}
async function handleOrderPaid(event, userId, env) {}
async function handleOrderRefunded(event, userId, env) {}
