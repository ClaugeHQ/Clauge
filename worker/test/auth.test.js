import { describe, it, expect, beforeEach } from "vitest";
import { env } from "cloudflare:test";
import { entitlementsForPlan } from "../src/auth.js";
import { seedUser } from "./setup.js";

describe("entitlementsForPlan", () => {
  it("free returns features all locked", () => {
    const ent = entitlementsForPlan("free");
    expect(ent.features.clauge_ai).toBe(false);
    expect(ent.features.unlimited_coworkers).toBe(false);
    expect(ent.features.premium_themes).toBe(false);
  });

  it("pro returns features unlocked", () => {
    const ent = entitlementsForPlan("pro");
    expect(ent.features.clauge_ai).toBe(true);
    expect(ent.features.unlimited_coworkers).toBe(true);
    expect(ent.features.premium_themes).toBe(true);
  });
});

describe("/api/auth/me response (smoke)", () => {
  beforeEach(async () => {
    await env.CLAUGE_DB.prepare("DELETE FROM users").run();
  });

  it("includes credits and entitlements", async () => {
    const userId = await seedUser({ slug: "u_me" });
    await env.CLAUGE_DB.prepare(
      `UPDATE users SET plan='pro', subscription_status='active',
         credit_allowance_per_cycle=1000, credits_remaining=600,
         current_period_end='2026-06-16T00:00:00Z'
       WHERE user_id=?`
    )
      .bind(userId)
      .run();
    const { buildMeResponse } = await import("../src/auth.js");
    const resp = await buildMeResponse(env, userId);
    expect(resp.status).toBe(200);
    const body = await resp.json();
    expect(body.plan).toBe("pro");
    expect(body.entitlements.features.clauge_ai).toBe(true);
    expect(body.entitlements.credits.remaining).toBe(600);
    expect(body.entitlements.credits.allowance).toBe(1000);
    expect(body.entitlements.credits.resets_at).toBe("2026-06-16T00:00:00Z");
  });
});
