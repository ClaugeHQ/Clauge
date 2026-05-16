import { describe, it, expect, beforeEach, vi } from "vitest";
import { env } from "cloudflare:test";
import { handleAiChat } from "../src/ai.js";
import { seedUser } from "./setup.js";

function ssePayload(chunks) {
  // chunks: array of objects, last one carries usage
  const parts = chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`);
  parts.push("data: [DONE]\n\n");
  return new ReadableStream({
    start(controller) {
      const enc = new TextEncoder();
      for (const p of parts) controller.enqueue(enc.encode(p));
      controller.close();
    },
  });
}

async function readAllText(resp) {
  const reader = resp.body.getReader();
  const dec = new TextDecoder();
  let buf = "";
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += dec.decode(value);
  }
  return buf;
}

describe("handleAiChat", () => {
  beforeEach(async () => {
    await env.CLAUGE_DB.prepare("DELETE FROM users").run();
    await env.CLAUGE_DB.prepare("DELETE FROM credit_usage_log").run();
    const list = await env.CLAUGE_KV.list({ prefix: "rl:" });
    for (const k of list.keys) await env.CLAUGE_KV.delete(k.name);
    const list2 = await env.CLAUGE_KV.list({ prefix: "burst:" });
    for (const k of list2.keys) await env.CLAUGE_KV.delete(k.name);
    await env.CLAUGE_KV.put("ai:credit_weights", '{"operations":{"chat":{"base":1,"long_ctx_threshold_tokens":8000,"long_ctx_multiplier":2}},"cost_to_clauge_credit_divisor_usd":0.01,"min_credits_per_call":1}');
    await env.CLAUGE_KV.put("ai:rate_limits", '{"per_user_rpm":30,"burst_budget_fraction":0.10,"burst_window_seconds":3600}');
  });

  it("returns 401 without user", async () => {
    const r = await handleAiChat(new Request("https://x", { method: "POST", body: "{}" }), env, null);
    expect(r.status).toBe(401);
  });

  it("returns 402 when user has no credits", async () => {
    const userId = await seedUser({ slug: "u_nocred" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', credit_allowance_per_cycle=1000, credits_remaining=0 WHERE user_id=?"
    )
      .bind(userId)
      .run();
    const r = await handleAiChat(
      new Request("https://x", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ messages: [{ role: "user", content: "hi" }], request_id: "11111111-1111-4111-8111-111111111111" }),
      }),
      env,
      userId
    );
    expect(r.status).toBe(402);
  });

  it("proxies SSE, strips model field, deducts credits", async () => {
    const userId = await seedUser({ slug: "u_chat_ok" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', credit_allowance_per_cycle=1000, credits_remaining=100 WHERE user_id=?"
    )
      .bind(userId)
      .run();
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(async () => {
      return new Response(
        ssePayload([
          { id: "c1", model: "family-a/model-x", choices: [{ delta: { content: "hi" } }] },
          { id: "c2", model: "family-a/model-x", choices: [{ delta: { content: "!" } }], usage: { prompt_tokens: 5, completion_tokens: 1, cost: 0.002 } },
        ]),
        { status: 200, headers: { "content-type": "text/event-stream" } }
      );
    });
    try {
      const r = await handleAiChat(
        new Request("https://x", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ messages: [{ role: "user", content: "ping" }], request_id: "22222222-2222-4222-8222-222222222222" }),
        }),
        env,
        userId
      );
      expect(r.status).toBe(200);
      const text = await readAllText(r);
      expect(text).not.toContain("family-a");
      expect(text).not.toContain("model-x");
      expect(text).toContain("hi");
      // Allow microtask for post-deduct
      await new Promise((r) => setTimeout(r, 50));
      const row = await env.CLAUGE_DB.prepare("SELECT credits_remaining FROM users WHERE user_id=?")
        .bind(userId).first();
      expect(row.credits_remaining).toBeLessThan(100);
    } finally {
      fetchMock.mockRestore();
    }
  });
});

describe("handleAiChat — request_id validation", () => {
  it("rejects non-UUID request_id with 400", async () => {
    const userId = await seedUser({ slug: "u_baduuid" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', credits_remaining=100 WHERE user_id=?"
    ).bind(userId).run();
    const r = await handleAiChat(
      new Request("https://x", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ messages: [{ role: "user", content: "hi" }], request_id: "not-a-uuid" }),
      }),
      env,
      userId
    );
    expect(r.status).toBe(400);
  });
});

describe("handleAiBalance", () => {
  it("returns 401 without user", async () => {
    const { handleAiBalance } = await import("../src/ai.js");
    const r = await handleAiBalance(env, null);
    expect(r.status).toBe(401);
  });

  it("returns balance + resets_at + allowance for pro user", async () => {
    const userId = await seedUser({ slug: "u_bal" });
    await env.CLAUGE_DB.prepare(
      "UPDATE users SET plan='pro', subscription_status='active', credit_allowance_per_cycle=1000, credits_remaining=420, current_period_end='2026-06-16T00:00:00Z' WHERE user_id=?"
    )
      .bind(userId)
      .run();
    const { handleAiBalance } = await import("../src/ai.js");
    const r = await handleAiBalance(env, userId);
    expect(r.status).toBe(200);
    const body = await r.json();
    expect(body.remaining).toBe(420);
    expect(body.allowance).toBe(1000);
    expect(body.resets_at).toBe("2026-06-16T00:00:00Z");
  });
});

describe("handleAiUsage", () => {
  it("returns 401 without user", async () => {
    const { handleAiUsage } = await import("../src/ai.js");
    const r = await handleAiUsage(env, null, new URL("https://x/api/ai/usage"));
    expect(r.status).toBe(401);
  });

  it("returns paginated rows newest-first", async () => {
    const userId = await seedUser({ slug: "u_usage" });
    for (let i = 0; i < 5; i++) {
      await env.CLAUGE_DB.prepare(
        `INSERT INTO credit_usage_log (user_id, operation, clauge_credits, cost_usd_micros, request_id, occurred_at)
         VALUES (?, ?, ?, ?, ?, datetime('now', ?))`
      )
        .bind(userId, "chat", i + 1, (i + 1) * 1000, `req_${i}`, `-${i} minutes`)
        .run();
    }
    const { handleAiUsage } = await import("../src/ai.js");
    const r = await handleAiUsage(env, userId, new URL("https://x/api/ai/usage?limit=3"));
    const body = await r.json();
    expect(body.entries.length).toBe(3);
    expect(body.entries[0].request_id).toBe("req_0"); // newest
  });
});
