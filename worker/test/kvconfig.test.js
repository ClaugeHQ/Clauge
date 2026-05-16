import { describe, it, expect, beforeEach } from "vitest";
import { env } from "cloudflare:test";
import { loadUpstreamPool, loadCreditWeights, loadRateLimits } from "../src/kvconfig.js";

describe("kvconfig", () => {
  beforeEach(async () => {
    await env.CLAUGE_KV.delete("ai:upstream_pool");
    await env.CLAUGE_KV.delete("ai:credit_weights");
    await env.CLAUGE_KV.delete("ai:rate_limits");
  });

  it("loadUpstreamPool returns parsed JSON when key present", async () => {
    await env.CLAUGE_KV.put("ai:upstream_pool", '{"model":"auto","allow":["family/*"]}');
    const pool = await loadUpstreamPool(env);
    expect(pool.model).toBe("auto");
    expect(pool.allow).toEqual(["family/*"]);
  });

  it("loadUpstreamPool returns empty placeholder when key absent (operator must seed)", async () => {
    const pool = await loadUpstreamPool(env);
    expect(pool.model).toBe("");
    expect(pool.allow).toEqual([]);
  });

  it("loadCreditWeights returns parsed JSON when key present", async () => {
    await env.CLAUGE_KV.put(
      "ai:credit_weights",
      '{"operations":{"chat":{"base":2}},"cost_to_clauge_credit_divisor_usd":0.02,"min_credits_per_call":1}'
    );
    const w = await loadCreditWeights(env);
    expect(w.operations.chat.base).toBe(2);
    expect(w.cost_to_clauge_credit_divisor_usd).toBe(0.02);
  });

  it("loadCreditWeights returns defaults when key absent", async () => {
    const w = await loadCreditWeights(env);
    expect(w.operations.chat.base).toBeGreaterThan(0);
    expect(w.min_credits_per_call).toBeGreaterThanOrEqual(1);
  });

  it("loadRateLimits returns defaults when key absent", async () => {
    const r = await loadRateLimits(env);
    expect(r.per_user_rpm).toBeGreaterThan(0);
    expect(r.burst_budget_fraction).toBeGreaterThan(0);
    expect(r.burst_window_seconds).toBeGreaterThan(0);
  });
});
