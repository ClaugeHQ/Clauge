import { describe, it, expect, beforeEach } from "vitest";
import { env } from "cloudflare:test";
import { sweepPastDue } from "../src/cron.js";
import { seedUser } from "./setup.js";

describe("sweepPastDue", () => {
  beforeEach(async () => {
    await env.CLAUGE_DB.prepare("DELETE FROM users").run();
  });

  it("revokes users past_due for more than 3 days", async () => {
    const userId = await seedUser({ slug: "u_old_pd" });
    await env.CLAUGE_DB.prepare(
      `UPDATE users SET plan='pro', subscription_status='past_due',
         past_due_started_at = datetime('now', '-4 days'),
         credits_remaining=500
       WHERE user_id = ?`
    )
      .bind(userId)
      .run();
    const swept = await sweepPastDue(env);
    expect(swept).toBe(1);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT plan, subscription_status, credits_remaining FROM users WHERE user_id=?"
    )
      .bind(userId)
      .first();
    expect(row.plan).toBe("free");
    expect(row.subscription_status).toBe("canceled");
    expect(row.credits_remaining).toBe(0);
  });

  it("leaves users past_due for less than 3 days alone", async () => {
    const userId = await seedUser({ slug: "u_new_pd" });
    await env.CLAUGE_DB.prepare(
      `UPDATE users SET plan='pro', subscription_status='past_due',
         past_due_started_at = datetime('now', '-1 day'),
         credits_remaining=400
       WHERE user_id = ?`
    )
      .bind(userId)
      .run();
    const swept = await sweepPastDue(env);
    expect(swept).toBe(0);
    const row = await env.CLAUGE_DB.prepare(
      "SELECT plan, subscription_status FROM users WHERE user_id=?"
    )
      .bind(userId)
      .first();
    expect(row.plan).toBe("pro");
    expect(row.subscription_status).toBe("past_due");
  });

  it("ignores active and free users entirely", async () => {
    const u1 = await seedUser({ slug: "u_active" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active' WHERE user_id=?"
    ).bind(u1).run();
    const u2 = await seedUser({ slug: "u_free" });
    const swept = await sweepPastDue(env);
    expect(swept).toBe(0);
  });
});
