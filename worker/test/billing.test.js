import { describe, it, expect, beforeEach } from "vitest";
import { env } from "cloudflare:test";
import { handleBillingWebhook } from "../src/billing.js";
import { seedUser } from "./setup.js";

async function postWebhook(body, sigHex) {
  return handleBillingWebhook(
    new Request("https://x/api/billing/webhook", {
      method: "POST",
      headers: { "webhook-signature": sigHex, "content-type": "application/json" },
      body,
    }),
    env
  );
}

async function signedSig(body) {
  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw",
    enc.encode(env.POLAR_WEBHOOK_SECRET),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"]
  );
  const mac = await crypto.subtle.sign("HMAC", key, enc.encode(body));
  return Array.from(new Uint8Array(mac))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

describe("handleBillingWebhook router", () => {
  beforeEach(async () => {
    await env.CLAUGE_DB.prepare("DELETE FROM subscription_history").run();
    await env.CLAUGE_DB.prepare("DELETE FROM users").run();
  });

  it("rejects requests with missing signature", async () => {
    const r = await handleBillingWebhook(
      new Request("https://x", { method: "POST", body: "{}" }),
      env
    );
    expect(r.status).toBe(401);
  });

  it("rejects requests with bad signature", async () => {
    const r = await postWebhook("{}", "deadbeef");
    expect(r.status).toBe(401);
  });

  it("rejects events older than 5 minutes", async () => {
    const body = JSON.stringify({
      id: "evt_old",
      type: "subscription.created",
      created_at: new Date(Date.now() - 6 * 60_000).toISOString(),
      data: {},
    });
    const sig = await signedSig(body);
    const r = await postWebhook(body, sig);
    expect(r.status).toBe(400);
  });

  it("returns 200 for an unknown event type (graceful drop)", async () => {
    const body = JSON.stringify({
      id: "evt_unknown",
      type: "some.future.event",
      created_at: new Date().toISOString(),
      data: {},
    });
    const sig = await signedSig(body);
    const r = await postWebhook(body, sig);
    expect(r.status).toBe(200);
  });

  it("dedupes by polar_event_id (replay-safe)", async () => {
    const userId = await seedUser({ slug: "u1" });
    const body = JSON.stringify({
      id: "evt_dup_1",
      type: "subscription.created",
      created_at: new Date().toISOString(),
      data: {
        id: "sub_test_1",
        status: "active",
        current_period_start: new Date().toISOString(),
        current_period_end: new Date(Date.now() + 30 * 86400_000).toISOString(),
        customer: { external_id: String(userId) },
        product: { prices: [{ id: env.POLAR_PRICE_MONTHLY }] },
        cancel_at_period_end: false,
      },
    });
    const sig = await signedSig(body);
    expect((await postWebhook(body, sig)).status).toBe(200);
    expect((await postWebhook(body, sig)).status).toBe(200);
    const count = await env.CLAUGE_DB.prepare(
      "SELECT COUNT(*) AS n FROM subscription_history WHERE polar_event_id = ?"
    )
      .bind("evt_dup_1")
      .first();
    expect(count.n).toBe(1);
  });
});
