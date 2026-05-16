import { loadUpstreamPool, loadCreditWeights, loadRateLimits } from "./kvconfig.js";
import { buildUpstreamRequest, callUpstream, sanitizeChunk, sanitizeFinalUsage } from "./upstream.js";
import { deductCredits, classifyOperation, computeChargeCredits, estimateTokens } from "./credits.js";
import { checkRpm, checkBurstBudget } from "./ratelimit.js";

const SSE_HEADERS = {
  "content-type": "text/event-stream",
  "cache-control": "no-cache, no-transform",
  "x-accel-buffering": "no",
};

function errResponse(code, message, status) {
  return new Response(JSON.stringify({ error: code, message, retryable: status >= 500 || code === "AI_BUSY" }), {
    status,
    headers: { "content-type": "application/json" },
  });
}

export async function handleAiChat(request, env, userId) {
  if (!userId) return errResponse("UNAUTHORIZED", "sign in required", 401);

  let body;
  try {
    body = await request.json();
  } catch {
    return errResponse("BAD_REQUEST", "invalid json", 400);
  }
  if (!Array.isArray(body.messages) || body.messages.length === 0) {
    return errResponse("BAD_REQUEST", "messages required", 400);
  }
  if (typeof body.request_id !== "string" || body.request_id.length < 8) {
    return errResponse("BAD_REQUEST", "request_id required (uuid)", 400);
  }

  const weights = await loadCreditWeights(env);
  const limits = await loadRateLimits(env);

  if (!(await checkRpm(userId, limits.per_user_rpm, env))) {
    return errResponse("RATE_LIMITED", "too many requests, slow down", 429);
  }

  const operation = classifyOperation(body);
  const estTokens = estimateTokens(body.messages);
  const estCharge = computeChargeCredits(operation, estTokens, 0, weights);

  const userRow = await env.CLAUGE_DB.prepare(
    "SELECT credit_allowance_per_cycle, credits_remaining, plan FROM users WHERE user_id = ?"
  )
    .bind(userId)
    .first();
  if (!userRow || userRow.plan !== "pro") {
    return errResponse("NOT_PRO", "Clauge AI requires Pro", 403);
  }
  if (userRow.credits_remaining < estCharge) {
    return errResponse("INSUFFICIENT_CREDITS", "out of Clauge AI credits this cycle", 402);
  }
  if (!(await checkBurstBudget(userId, userRow.credit_allowance_per_cycle, limits.burst_budget_fraction, limits.burst_window_seconds, estCharge, env))) {
    return errResponse("BURST_LIMITED", "using credits too quickly, try in an hour", 429);
  }

  const pool = await loadUpstreamPool(env);
  if (!pool.model) {
    return errResponse("AI_UNAVAILABLE", "Clauge AI is not configured", 503);
  }
  const upReq = buildUpstreamRequest({
    messages: body.messages,
    pool,
    systemSuffix: typeof body.system === "string" ? body.system : "",
  });
  let upResp;
  try {
    upResp = await callUpstream(upReq, env);
  } catch {
    return errResponse("AI_BUSY", "Clauge AI is busy, retry in a moment", 503);
  }
  if (!upResp.ok || !upResp.body) {
    return errResponse("AI_BUSY", "Clauge AI is busy, retry in a moment", 503);
  }

  const { readable, writable } = new TransformStream();
  const writer = writable.getWriter();
  const dec = new TextDecoder();
  const enc = new TextEncoder();
  let finalUsage = { prompt_tokens: estTokens, completion_tokens: 0, cost_usd_micros: 0 };
  let buf = "";

  (async () => {
    try {
      const reader = upResp.body.getReader();
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += dec.decode(value, { stream: true });
        let idx;
        while ((idx = buf.indexOf("\n\n")) >= 0) {
          const frame = buf.slice(0, idx);
          buf = buf.slice(idx + 2);
          if (!frame.startsWith("data:")) continue;
          const dataStr = frame.slice(5).trim();
          if (dataStr === "[DONE]") {
            await writer.write(enc.encode("data: [DONE]\n\n"));
            continue;
          }
          try {
            const obj = JSON.parse(dataStr);
            if (obj?.usage) finalUsage = sanitizeFinalUsage(obj);
            const clean = sanitizeChunk(obj);
            await writer.write(enc.encode("data: " + JSON.stringify(clean) + "\n\n"));
          } catch {
            // Skip malformed frame silently
          }
        }
      }
    } catch {
      // Stream interrupted — client will retry. Still settle credits.
    } finally {
      await writer.close();
      const charge = computeChargeCredits(operation, estTokens, finalUsage.cost_usd_micros, weights);
      await deductCredits(
        userId,
        {
          operation,
          clauge_credits: charge,
          cost_usd_micros: finalUsage.cost_usd_micros,
          request_id: body.request_id,
        },
        env
      );
    }
  })();

  return new Response(readable, { status: 200, headers: SSE_HEADERS });
}

export async function handleAiBalance(env, userId) {
  if (!userId) return errResponse("UNAUTHORIZED", "sign in required", 401);
  const row = await env.CLAUGE_DB.prepare(
    "SELECT credits_remaining, credit_allowance_per_cycle, current_period_end FROM users WHERE user_id = ?"
  )
    .bind(userId)
    .first();
  if (!row) return errResponse("NOT_FOUND", "user not found", 404);
  return new Response(
    JSON.stringify({
      remaining: row.credits_remaining,
      allowance: row.credit_allowance_per_cycle,
      resets_at: row.current_period_end,
    }),
    { status: 200, headers: { "content-type": "application/json" } }
  );
}

export async function handleAiUsage(env, userId, url) {
  if (!userId) return errResponse("UNAUTHORIZED", "sign in required", 401);
  const limit = Math.min(Number(url.searchParams.get("limit") ?? 50), 200);
  const before = url.searchParams.get("before"); // ISO timestamp
  const stmt = before
    ? env.CLAUGE_DB.prepare(
        `SELECT occurred_at, operation, clauge_credits, cost_usd_micros, request_id
           FROM credit_usage_log
          WHERE user_id = ? AND occurred_at < ?
          ORDER BY occurred_at DESC LIMIT ?`
      ).bind(userId, before, limit)
    : env.CLAUGE_DB.prepare(
        `SELECT occurred_at, operation, clauge_credits, cost_usd_micros, request_id
           FROM credit_usage_log
          WHERE user_id = ?
          ORDER BY occurred_at DESC LIMIT ?`
      ).bind(userId, limit);
  const { results } = await stmt.all();
  return new Response(
    JSON.stringify({
      entries: results,
      next_before: results.length === limit ? results[results.length - 1].occurred_at : null,
    }),
    { status: 200, headers: { "content-type": "application/json" } }
  );
}
