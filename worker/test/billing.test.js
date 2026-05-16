import { describe, it, expect, beforeEach, vi } from "vitest";
import { env } from "cloudflare:test";
import { handleBillingWebhook } from "../src/billing.js";
import { seedUser } from "./setup.js";

async function buildSignedHeaders(rawBody, opts = {}) {
  const enc = new TextEncoder();
  const id = opts.id ?? `msg_${Math.random().toString(36).slice(2)}`;
  const timestamp = opts.timestamp ?? String(Math.floor(Date.now() / 1000));
  const secret = env.POLAR_WEBHOOK_SECRET;
  const key = await crypto.subtle.importKey(
    "raw",
    enc.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"]
  );
  const mac = await crypto.subtle.sign("HMAC", key, enc.encode(`${id}.${timestamp}.${rawBody}`));
  const sigBytes = new Uint8Array(mac);
  let bin = "";
  for (const b of sigBytes) bin += String.fromCharCode(b);
  return { id, headers: new Headers({
    "webhook-id": id,
    "webhook-timestamp": timestamp,
    "webhook-signature": `v1,${btoa(bin)}`,
    "content-type": "application/json",
  }) };
}

async function postWebhook(body, opts = {}) {
  const { headers } = await buildSignedHeaders(body, opts);
  return handleBillingWebhook(
    new Request("https://x/api/billing/webhook", { method: "POST", headers, body }),
    env
  );
}

describe("handleBillingWebhook router", () => {
  beforeEach(async () => {
    await env.CLAUGE_DB.prepare("DELETE FROM subscription_history").run();
    await env.CLAUGE_DB.prepare("DELETE FROM users").run();
  });

  it("rejects requests with missing signature headers", async () => {
    const r = await handleBillingWebhook(
      new Request("https://x", { method: "POST", body: "{}" }),
      env
    );
    expect(r.status).toBe(401);
  });

  it("rejects requests with bad signature", async () => {
    const body = "{}";
    const r = await handleBillingWebhook(
      new Request("https://x", {
        method: "POST",
        headers: {
          "webhook-id": "msg_x",
          "webhook-timestamp": String(Math.floor(Date.now() / 1000)),
          "webhook-signature": "v1,YmFkc2lndmFsdWU=",
        },
        body,
      }),
      env
    );
    expect(r.status).toBe(401);
  });

  it("rejects requests with stale timestamp header", async () => {
    const body = JSON.stringify({ type: "subscription.created", data: {} });
    const old = String(Math.floor(Date.now() / 1000) - 6 * 60);
    const r = await postWebhook(body, { timestamp: old });
    expect(r.status).toBe(401);
  });

  it("returns 200 for an unknown event type (graceful drop)", async () => {
    const body = JSON.stringify({
      type: "some.future.event",
      data: {},
    });
    const r = await postWebhook(body);
    expect(r.status).toBe(200);
  });

  it("dedupes by webhook-id (replay-safe)", async () => {
    const userId = await seedUser({ slug: "u1" });
    const body = JSON.stringify({
      type: "subscription.created",
      data: {
        id: "sub_test_1",
        status: "active",
        current_period_start: new Date().toISOString(),
        current_period_end: new Date(Date.now() + 30 * 86400_000).toISOString(),
        external_customer_id: String(userId),
        product: { prices: [{ id: env.POLAR_PRODUCT_MONTHLY }] },
        cancel_at_period_end: false,
      },
    });
    // Two calls with the same webhook-id header
    expect((await postWebhook(body, { id: "msg_dup_1" })).status).toBe(200);
    expect((await postWebhook(body, { id: "msg_dup_1" })).status).toBe(200);
    const count = await env.CLAUGE_DB.prepare(
      "SELECT COUNT(*) AS n FROM subscription_history WHERE polar_event_id = ?"
    ).bind("msg_dup_1").first();
    expect(count.n).toBe(1);
  });
});

describe("subscription.created handler", () => {
  it("flips plan to pro, sets period bounds, grants credits", async () => {
    const userId = await seedUser({ slug: "u_created" });
    const body = JSON.stringify({
      type: "subscription.created",
      data: {
        id: "sub_abc",
        status: "active",
        current_period_start: "2026-05-16T00:00:00Z",
        current_period_end: "2026-06-16T00:00:00Z",
        cancel_at_period_end: false,
        external_customer_id: String(userId),
        product: { prices: [{ id: env.POLAR_PRODUCT_MONTHLY }] },
      },
    });
    await postWebhook(body);
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
      type: "subscription.canceled",
      data: {
        id: "sub_c",
        status: "active",
        cancel_at_period_end: true,
        current_period_end: "2026-06-16T00:00:00Z",
        external_customer_id: String(userId),
      },
    });
    await postWebhook(body);
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
      type: "subscription.revoked",
      data: { id: "sub_r", status: "canceled", external_customer_id: String(userId) },
    });
    await postWebhook(body);
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
      type: "subscription.updated",
      data: {
        id: "sub_p",
        status: "past_due",
        cancel_at_period_end: false,
        current_period_end: "2026-06-16T00:00:00Z",
        external_customer_id: String(userId),
      },
    });
    await postWebhook(body);
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
      type: "subscription.updated",
      data: {
        id: "sub_u",
        status: "unpaid",
        external_customer_id: String(userId),
      },
    });
    await postWebhook(body);
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
      type: "order.paid",
      data: {
        id: "ord_p1",
        status: "paid",
        subscription_id: "sub_p1",
        billing_reason: "subscription_cycle",
        current_period_start: "2026-06-16T00:00:00Z",
        current_period_end: "2026-07-16T00:00:00Z",
        external_customer_id: String(userId),
      },
    });
    await postWebhook(body);
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
      type: "order.paid",
      data: {
        id: "ord_y1",
        subscription_id: "sub_y1",
        current_period_start: "2027-05-16T00:00:00Z",
        current_period_end: "2028-05-16T00:00:00Z",
        external_customer_id: String(userId),
      },
    });
    await postWebhook(body);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT credits_remaining FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.credits_remaining).toBe(12000);
  });
});

describe("order.paid handler — derives period from user row not payload", () => {
  it("grants 12x for yearly even when order payload omits period bounds", async () => {
    const userId = await seedUser({ slug: "u_yearly_no_bounds" });
    // Simulate state after subscription.created already fired and set bounds:
    await env.CLAUGE_DB.prepare(
      `UPDATE users SET plan='pro', subscription_status='active',
         polar_subscription_id='sub_y2',
         current_period_start='2026-05-16T00:00:00Z',
         current_period_end='2027-05-16T00:00:00Z',
         credit_allowance_per_cycle=1000, credits_remaining=42
       WHERE user_id=?`
    )
      .bind(userId)
      .run();
    // order.paid payload WITHOUT current_period_start/current_period_end:
    const body = JSON.stringify({
      type: "order.paid",
      data: {
        id: "ord_y2",
        subscription_id: "sub_y2",
        external_customer_id: String(userId),
        // NOTE: no current_period_start or current_period_end here
      },
    });
    await postWebhook(body);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT credits_remaining FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.credits_remaining).toBe(12000);
  });

  it("grants 1x for monthly even when order payload omits period bounds", async () => {
    const userId = await seedUser({ slug: "u_monthly_no_bounds" });
    await env.CLAUGE_DB.prepare(
      `UPDATE users SET plan='pro', subscription_status='active',
         polar_subscription_id='sub_m2',
         current_period_start='2026-05-16T00:00:00Z',
         current_period_end='2026-06-16T00:00:00Z',
         credit_allowance_per_cycle=1000, credits_remaining=0
       WHERE user_id=?`
    )
      .bind(userId)
      .run();
    const body = JSON.stringify({
      type: "order.paid",
      data: {
        id: "ord_m2",
        subscription_id: "sub_m2",
        external_customer_id: String(userId),
      },
    });
    await postWebhook(body);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT credits_remaining FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.credits_remaining).toBe(1000);
  });
});

describe("initial purchase flow (sub.created + order.paid)", () => {
  it("grants credits exactly once across both events", async () => {
    const userId = await seedUser({ slug: "u_initial" });

    // 1. subscription.created fires (no credits granted)
    const subBody = JSON.stringify({
      type: "subscription.created",
      data: {
        id: "sub_init",
        status: "active",
        cancel_at_period_end: false,
        current_period_start: "2026-05-16T00:00:00Z",
        current_period_end: "2026-06-16T00:00:00Z",
        external_customer_id: String(userId),
        product: { prices: [{ id: env.POLAR_PRODUCT_MONTHLY }] },
      },
    });
    await postWebhook(subBody);
    let row = await env.CLAUGE_DB.prepare(
      "SELECT plan, credits_remaining, credit_allowance_per_cycle FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.plan).toBe("pro");
    expect(row.credit_allowance_per_cycle).toBe(1000);
    expect(row.credits_remaining).toBe(0); // not yet granted

    // 2. order.paid fires (grants 1× allowance for monthly)
    const ordBody = JSON.stringify({
      type: "order.paid",
      data: {
        id: "ord_init",
        subscription_id: "sub_init",
        current_period_start: "2026-05-16T00:00:00Z",
        current_period_end: "2026-06-16T00:00:00Z",
        external_customer_id: String(userId),
      },
    });
    await postWebhook(ordBody);
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
      type: "order.refunded",
      data: { id: "ord_r", external_customer_id: String(userId) },
    });
    await postWebhook(body);
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

  it("returns schema_version=1 and seeded plans with discount=null", async () => {
    const { handleGetPricing } = await import("../src/billing.js");
    const r = await handleGetPricing(env);
    expect(r.status).toBe(200);
    expect(r.headers.get("cache-control")).toContain("max-age=300");
    const body = await r.json();
    expect(body.schema_version).toBe(1);
    expect(body.plans.map((p) => p.id).sort()).toEqual(["monthly", "yearly"]);
    const monthly = body.plans.find((p) => p.id === "monthly");
    const yearly  = body.plans.find((p) => p.id === "yearly");
    expect(monthly).toEqual({ id: "monthly", price_usd: 15, discount: null });
    expect(yearly).toEqual({ id: "yearly", price_usd: 150, discount: null });
  });

  it("attaches per-plan discount when row exists", async () => {
    await env.CLAUGE_DB.prepare(
      "INSERT OR REPLACE INTO billing_discount (plan_id, percent, code) VALUES ('yearly', 53, 'INTRO53')"
    ).run();
    const { handleGetPricing } = await import("../src/billing.js");
    const r = await handleGetPricing(env);
    const body = await r.json();
    const yearly = body.plans.find((p) => p.id === "yearly");
    expect(yearly.discount).toEqual({ percent: 53, code: "INTRO53" });
    // Monthly stays null
    expect(body.plans.find((p) => p.id === "monthly").discount).toBeNull();
  });

  it("supports different discounts per plan", async () => {
    await env.CLAUGE_DB.prepare(
      "INSERT OR REPLACE INTO billing_discount (plan_id, percent, code) VALUES ('monthly', 53, 'INTRO53M')"
    ).run();
    await env.CLAUGE_DB.prepare(
      "INSERT OR REPLACE INTO billing_discount (plan_id, percent, code) VALUES ('yearly', 30, 'YEAR30')"
    ).run();
    const { handleGetPricing } = await import("../src/billing.js");
    const r = await handleGetPricing(env);
    const body = await r.json();
    expect(body.plans.find((p) => p.id === "monthly").discount).toEqual({ percent: 53, code: "INTRO53M" });
    expect(body.plans.find((p) => p.id === "yearly").discount).toEqual({ percent: 30, code: "YEAR30" });
  });

  it("supports auto-apply discount (code is null)", async () => {
    await env.CLAUGE_DB.prepare(
      "INSERT OR REPLACE INTO billing_discount (plan_id, percent, code) VALUES ('yearly', 20, NULL)"
    ).run();
    const { handleGetPricing } = await import("../src/billing.js");
    const r = await handleGetPricing(env);
    const body = await r.json();
    const yearly = body.plans.find((p) => p.id === "yearly");
    expect(yearly.discount).toEqual({ percent: 20, code: null });
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
      expect(body.products).toEqual([env.POLAR_PRODUCT_MONTHLY]);
      expect(body.external_customer_id).toBe(String(userId));
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

describe("polar_customer_id capture", () => {
  it("subscription.created sets polar_customer_id from data.customer_id", async () => {
    const userId = await seedUser({ slug: "u_cust_id" });
    const body = JSON.stringify({
      type: "subscription.created",
      data: {
        id: "sub_cust_1",
        status: "active",
        cancel_at_period_end: false,
        current_period_start: "2026-05-16T00:00:00Z",
        current_period_end: "2026-06-16T00:00:00Z",
        external_customer_id: String(userId),
        customer_id: "cust_polar_xyz",
        product: { prices: [{ id: env.POLAR_PRODUCT_MONTHLY }] },
      },
    });
    await postWebhook(body);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT polar_customer_id FROM users WHERE user_id = ?"
    ).bind(userId).first();
    expect(row.polar_customer_id).toBe("cust_polar_xyz");
  });

  it("subscription.created accepts customer_id at data.customer.id (nested)", async () => {
    const userId = await seedUser({ slug: "u_cust_id_nested" });
    const body = JSON.stringify({
      type: "subscription.created",
      data: {
        id: "sub_cust_2",
        status: "active",
        cancel_at_period_end: false,
        current_period_start: "2026-05-16T00:00:00Z",
        current_period_end: "2026-06-16T00:00:00Z",
        external_customer_id: String(userId),
        customer: { id: "cust_polar_abc", external_id: String(userId) },
      },
    });
    await postWebhook(body);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT polar_customer_id FROM users WHERE user_id = ?"
    ).bind(userId).first();
    expect(row.polar_customer_id).toBe("cust_polar_abc");
  });
});

describe("subscription.uncanceled dispatch", () => {
  it("clears cancel_at_period_end when user un-cancels via portal", async () => {
    const userId = await seedUser({ slug: "u_uncancel" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', cancel_at_period_end=1, polar_subscription_id='sub_unc' WHERE user_id=?"
    ).bind(userId).run();
    const body = JSON.stringify({
      type: "subscription.uncanceled",
      data: {
        id: "sub_unc",
        status: "active",
        cancel_at_period_end: false,
        external_customer_id: String(userId),
      },
    });
    await postWebhook(body);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT cancel_at_period_end, subscription_status FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.cancel_at_period_end).toBe(0);
    expect(row.subscription_status).toBe("active");
  });
});

describe("subscription.active dispatch", () => {
  it("reconciles status to active after past_due recovery", async () => {
    const userId = await seedUser({ slug: "u_active" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='past_due', past_due_started_at=datetime('now','-1 day'), polar_subscription_id='sub_act' WHERE user_id=?"
    ).bind(userId).run();
    const body = JSON.stringify({
      type: "subscription.active",
      data: {
        id: "sub_act",
        status: "active",
        cancel_at_period_end: false,
        external_customer_id: String(userId),
      },
    });
    await postWebhook(body);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT subscription_status, past_due_started_at FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.subscription_status).toBe("active");
    expect(row.past_due_started_at).toBeNull();
  });
});

describe("subscription.past_due dispatch", () => {
  it("stamps past_due_started_at when explicit past_due event fires", async () => {
    const userId = await seedUser({ slug: "u_past_due_explicit" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', polar_subscription_id='sub_pde' WHERE user_id=?"
    ).bind(userId).run();
    const body = JSON.stringify({
      type: "subscription.past_due",
      data: {
        id: "sub_pde",
        status: "past_due",
        cancel_at_period_end: false,
        external_customer_id: String(userId),
      },
    });
    await postWebhook(body);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT subscription_status, past_due_started_at FROM users WHERE user_id=?"
    ).bind(userId).first();
    expect(row.subscription_status).toBe("past_due");
    expect(row.past_due_started_at).not.toBeNull();
  });
});
