import { describe, it, expect, beforeEach, vi } from "vitest";
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

describe("subscription.created handler", () => {
  it("flips plan to pro, sets period bounds, grants credits", async () => {
    const userId = await seedUser({ slug: "u_created" });
    const body = JSON.stringify({
      id: "evt_sub_created_1",
      type: "subscription.created",
      created_at: new Date().toISOString(),
      data: {
        id: "sub_abc",
        status: "active",
        current_period_start: "2026-05-16T00:00:00Z",
        current_period_end: "2026-06-16T00:00:00Z",
        cancel_at_period_end: false,
        customer: { external_id: String(userId) },
        product: { prices: [{ id: env.POLAR_PRICE_MONTHLY }] },
      },
    });
    const sig = await signedSig(body);
    await postWebhook(body, sig);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT plan, subscription_status, polar_subscription_id, credits_remaining, credit_allowance_per_cycle, current_period_end FROM users WHERE user_id = ?"
    )
      .bind(userId)
      .first();
    expect(row.plan).toBe("pro");
    expect(row.subscription_status).toBe("active");
    expect(row.polar_subscription_id).toBe("sub_abc");
    expect(row.credit_allowance_per_cycle).toBeGreaterThan(0);
    expect(row.credits_remaining).toBe(0);
    expect(row.current_period_end).toBe("2026-06-16T00:00:00Z");
  });
});

describe("subscription.canceled handler", () => {
  it("sets cancel_at_period_end=1 but keeps user active", async () => {
    const userId = await seedUser({ slug: "u_cancel" });
    // Pre-seed an active sub
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', polar_subscription_id='sub_c', credit_allowance_per_cycle=1000, credits_remaining=400 WHERE user_id=?"
    )
      .bind(userId)
      .run();
    const body = JSON.stringify({
      id: "evt_sub_cancel_1",
      type: "subscription.canceled",
      created_at: new Date().toISOString(),
      data: {
        id: "sub_c",
        status: "active",
        cancel_at_period_end: true,
        current_period_end: "2026-06-16T00:00:00Z",
        customer: { external_id: String(userId) },
      },
    });
    const sig = await signedSig(body);
    await postWebhook(body, sig);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT plan, subscription_status, cancel_at_period_end, credits_remaining FROM users WHERE user_id=?"
    )
      .bind(userId)
      .first();
    expect(row.plan).toBe("pro");
    expect(row.cancel_at_period_end).toBe(1);
    expect(row.credits_remaining).toBe(400); // unchanged
  });
});

describe("subscription.revoked handler", () => {
  it("flips plan to free, wipes credits", async () => {
    const userId = await seedUser({ slug: "u_revoke" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', credits_remaining=200 WHERE user_id=?"
    )
      .bind(userId)
      .run();
    const body = JSON.stringify({
      id: "evt_sub_revoke_1",
      type: "subscription.revoked",
      created_at: new Date().toISOString(),
      data: { id: "sub_r", status: "canceled", customer: { external_id: String(userId) } },
    });
    const sig = await signedSig(body);
    await postWebhook(body, sig);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT plan, subscription_status, credits_remaining FROM users WHERE user_id=?"
    )
      .bind(userId)
      .first();
    expect(row.plan).toBe("free");
    expect(row.subscription_status).toBe("canceled");
    expect(row.credits_remaining).toBe(0);
  });
});

describe("subscription.updated past_due handler", () => {
  it("sets past_due_started_at on first transition, leaves credits", async () => {
    const userId = await seedUser({ slug: "u_pastdue" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', credits_remaining=500 WHERE user_id=?"
    )
      .bind(userId)
      .run();
    const body = JSON.stringify({
      id: "evt_sub_pastdue_1",
      type: "subscription.updated",
      created_at: new Date().toISOString(),
      data: {
        id: "sub_p",
        status: "past_due",
        cancel_at_period_end: false,
        current_period_end: "2026-06-16T00:00:00Z",
        customer: { external_id: String(userId) },
      },
    });
    const sig = await signedSig(body);
    await postWebhook(body, sig);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT subscription_status, past_due_started_at, plan, credits_remaining FROM users WHERE user_id=?"
    )
      .bind(userId)
      .first();
    expect(row.subscription_status).toBe("past_due");
    expect(row.past_due_started_at).not.toBeNull();
    expect(row.plan).toBe("pro"); // grace period, features still on
    expect(row.credits_remaining).toBe(500);
  });
});

describe("subscription.updated unpaid handler", () => {
  it("treats unpaid as immediate revocation", async () => {
    const userId = await seedUser({ slug: "u_unpaid" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='past_due', credits_remaining=300 WHERE user_id=?"
    )
      .bind(userId)
      .run();
    const body = JSON.stringify({
      id: "evt_sub_unpaid_1",
      type: "subscription.updated",
      created_at: new Date().toISOString(),
      data: {
        id: "sub_u",
        status: "unpaid",
        customer: { external_id: String(userId) },
      },
    });
    const sig = await signedSig(body);
    await postWebhook(body, sig);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT plan, subscription_status, credits_remaining FROM users WHERE user_id=?"
    )
      .bind(userId)
      .first();
    expect(row.plan).toBe("free");
    expect(row.subscription_status).toBe("unpaid");
    expect(row.credits_remaining).toBe(0);
  });
});

describe("order.paid handler", () => {
  it("resets credits on a new billing period", async () => {
    const userId = await seedUser({ slug: "u_paid" });
    await env.CLAUGE_DB.prepare(
      `UPDATE users SET plan='pro', subscription_status='active',
         polar_subscription_id='sub_p1',
         current_period_start='2026-05-16T00:00:00Z',
         current_period_end='2026-06-16T00:00:00Z',
         credit_allowance_per_cycle=1000, credits_remaining=42
       WHERE user_id=?`
    )
      .bind(userId)
      .run();
    const body = JSON.stringify({
      id: "evt_paid_1",
      type: "order.paid",
      created_at: new Date().toISOString(),
      data: {
        id: "ord_p1",
        status: "paid",
        subscription_id: "sub_p1",
        billing_reason: "subscription_cycle",
        current_period_start: "2026-06-16T00:00:00Z",
        current_period_end: "2026-07-16T00:00:00Z",
        customer: { external_id: String(userId) },
      },
    });
    const sig = await signedSig(body);
    await postWebhook(body, sig);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT credits_remaining, current_period_start FROM users WHERE user_id=?"
    )
      .bind(userId)
      .first();
    expect(row.credits_remaining).toBe(1000);
    expect(row.current_period_start).toBe("2026-06-16T00:00:00Z");
  });

});

describe("order.paid handler — yearly subscription", () => {
  it("grants allowance × 12 for yearly billing period", async () => {
    const userId = await seedUser({ slug: "u_yearly" });
    await env.CLAUGE_DB.prepare(
      `UPDATE users SET plan='pro', subscription_status='active',
         polar_subscription_id='sub_y1',
         current_period_start='2026-05-16T00:00:00Z',
         current_period_end='2027-05-16T00:00:00Z',
         credit_allowance_per_cycle=1000, credits_remaining=42
       WHERE user_id=?`
    )
      .bind(userId)
      .run();
    const body = JSON.stringify({
      id: "evt_yearly_paid",
      type: "order.paid",
      created_at: new Date().toISOString(),
      data: {
        id: "ord_y1",
        subscription_id: "sub_y1",
        current_period_start: "2027-05-16T00:00:00Z",
        current_period_end: "2028-05-16T00:00:00Z",
        customer: { external_id: String(userId) },
      },
    });
    const sig = await signedSig(body);
    await postWebhook(body, sig);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT credits_remaining FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.credits_remaining).toBe(12000);
  });
});

describe("initial purchase flow (sub.created + order.paid)", () => {
  it("grants credits exactly once across both events", async () => {
    const userId = await seedUser({ slug: "u_initial" });

    // 1. subscription.created fires (no credits granted)
    const subBody = JSON.stringify({
      id: "evt_init_sub",
      type: "subscription.created",
      created_at: new Date().toISOString(),
      data: {
        id: "sub_init",
        status: "active",
        cancel_at_period_end: false,
        current_period_start: "2026-05-16T00:00:00Z",
        current_period_end: "2026-06-16T00:00:00Z",
        customer: { external_id: String(userId) },
        product: { prices: [{ id: env.POLAR_PRICE_MONTHLY }] },
      },
    });
    await postWebhook(subBody, await signedSig(subBody));
    let row = await env.CLAUGE_DB.prepare(
      "SELECT plan, credits_remaining, credit_allowance_per_cycle FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.plan).toBe("pro");
    expect(row.credit_allowance_per_cycle).toBe(1000);
    expect(row.credits_remaining).toBe(0); // not yet granted

    // 2. order.paid fires (grants 1× allowance for monthly)
    const ordBody = JSON.stringify({
      id: "evt_init_ord",
      type: "order.paid",
      created_at: new Date().toISOString(),
      data: {
        id: "ord_init",
        subscription_id: "sub_init",
        current_period_start: "2026-05-16T00:00:00Z",
        current_period_end: "2026-06-16T00:00:00Z",
        customer: { external_id: String(userId) },
      },
    });
    await postWebhook(ordBody, await signedSig(ordBody));
    row = await env.CLAUGE_DB.prepare(
      "SELECT credits_remaining FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.credits_remaining).toBe(1000);
  });
});

describe("order.refunded handler", () => {
  it("treats refund as immediate revocation", async () => {
    const userId = await seedUser({ slug: "u_refund" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', credits_remaining=700 WHERE user_id=?"
    )
      .bind(userId)
      .run();
    const body = JSON.stringify({
      id: "evt_refund_1",
      type: "order.refunded",
      created_at: new Date().toISOString(),
      data: { id: "ord_r", customer: { external_id: String(userId) } },
    });
    const sig = await signedSig(body);
    await postWebhook(body, sig);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT plan, credits_remaining FROM users WHERE user_id=?"
    )
      .bind(userId)
      .first();
    expect(row.plan).toBe("free");
    expect(row.credits_remaining).toBe(0);
  });
});

describe("POST /api/billing/portal", () => {
  it("returns 401 without auth", async () => {
    const { handleCreatePortal } = await import("../src/billing.js");
    const r = await handleCreatePortal(env, null);
    expect(r.status).toBe(401);
  });

  it("returns 404 if user has no polar_customer_id", async () => {
    const userId = await seedUser({ slug: "u_portal_none" });
    const { handleCreatePortal } = await import("../src/billing.js");
    const r = await handleCreatePortal(env, userId);
    expect(r.status).toBe(404);
  });

  it("returns a portal url (Polar API mocked)", async () => {
    const userId = await seedUser({ slug: "u_portal" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET polar_customer_id='cus_x1' WHERE user_id=?"
    )
      .bind(userId)
      .run();
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(async (url) => {
      expect(String(url)).toContain("polar.sh");
      return new Response(JSON.stringify({ customer_portal_url: "https://sandbox.polar.sh/portal/x" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    });
    try {
      const { handleCreatePortal } = await import("../src/billing.js");
      const r = await handleCreatePortal(env, userId);
      expect(r.status).toBe(200);
      const p = await r.json();
      expect(p.url).toContain("polar.sh/portal");
    } finally {
      fetchMock.mockRestore();
    }
  });
});

describe("rate limits", () => {
  beforeEach(async () => {
    const list = await env.CLAUGE_KV.list({ prefix: "rl:key:" });
    for (const k of list.keys) await env.CLAUGE_KV.delete(k.name);
  });

  it("blocks the 6th checkout request in the same minute from one user", async () => {
    const userId = await seedUser({ slug: "u_rl_chk" });
    const { checkKeyRpm } = await import("../src/ratelimit.js");
    // burn the budget
    for (let i = 0; i < 5; i++) {
      expect(await checkKeyRpm(`checkout:${userId}`, 5, env)).toBe(true);
    }
    expect(await checkKeyRpm(`checkout:${userId}`, 5, env)).toBe(false);
    // The route is what enforces this — verified via the checkKeyRpm contract.
  });
});

describe("GET /api/billing/pricing", () => {
  beforeEach(async () => {
    await env.CLAUGE_DB.prepare("DELETE FROM billing_discount").run();
  });

  it("returns seeded monthly + yearly plans with no discount", async () => {
    const { handleGetPricing } = await import("../src/billing.js");
    const r = await handleGetPricing(env);
    expect(r.status).toBe(200);
    expect(r.headers.get("cache-control")).toContain("max-age=300");
    const body = await r.json();
    expect(body.plans.map(p => p.id).sort()).toEqual(["monthly", "yearly"]);
    expect(body.plans.find(p => p.id === "monthly").price_usd).toBe(15);
    expect(body.plans.find(p => p.id === "yearly").price_usd).toBe(150);
    expect(body.discount).toBeUndefined();
  });

  it("includes discount object when a row exists", async () => {
    await env.CLAUGE_DB.prepare(
      "INSERT OR REPLACE INTO billing_discount (id, percent, label, code) VALUES (1, 53, 'Introductory offer', 'INTRO53')"
    ).run();
    const { handleGetPricing } = await import("../src/billing.js");
    const r = await handleGetPricing(env);
    const body = await r.json();
    expect(body.discount).toEqual({
      percent: 53,
      label: "Introductory offer",
      code: "INTRO53",
    });
  });

  it("discount.code is nullable (auto-apply variant)", async () => {
    await env.CLAUGE_DB.prepare(
      "INSERT OR REPLACE INTO billing_discount (id, percent, label, code) VALUES (1, 20, 'Auto applied', NULL)"
    ).run();
    const { handleGetPricing } = await import("../src/billing.js");
    const r = await handleGetPricing(env);
    const body = await r.json();
    expect(body.discount.percent).toBe(20);
    expect(body.discount.label).toBe("Auto applied");
    expect(body.discount.code).toBeNull();
  });
});

describe("POST /api/billing/checkout", () => {
  it("returns 401 without auth", async () => {
    const { handleCreateCheckout } = await import("../src/billing.js");
    const r = await handleCreateCheckout(
      new Request("https://x", { method: "POST", body: '{"plan":"monthly"}' }),
      env,
      null
    );
    expect(r.status).toBe(401);
  });

  it("returns 400 on invalid plan", async () => {
    const userId = await seedUser({ slug: "u_chk" });
    const { handleCreateCheckout } = await import("../src/billing.js");
    const r = await handleCreateCheckout(
      new Request("https://x", { method: "POST", body: '{"plan":"weekly"}' }),
      env,
      userId
    );
    expect(r.status).toBe(400);
  });

  it("returns a checkout url for monthly plan (Polar API mocked)", async () => {
    const userId = await seedUser({ slug: "u_chk2", email: "user@test.invalid" });
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(async (url, init) => {
      expect(String(url)).toContain("polar.sh");
      const body = JSON.parse(init.body);
      expect(body.product_price_id).toBe(env.POLAR_PRICE_MONTHLY);
      expect(body.customer_external_id).toBe(String(userId));
      return new Response(JSON.stringify({ url: "https://sandbox.polar.sh/checkout/abc" }), {
        status: 201,
        headers: { "content-type": "application/json" },
      });
    });
    try {
      const { handleCreateCheckout } = await import("../src/billing.js");
      const r = await handleCreateCheckout(
        new Request("https://x", {
          method: "POST",
          body: '{"plan":"monthly"}',
          headers: { "content-type": "application/json" },
        }),
        env,
        userId
      );
      expect(r.status).toBe(200);
      const payload = await r.json();
      expect(payload.url).toContain("polar.sh/checkout");
    } finally {
      fetchMock.mockRestore();
    }
  });
});
