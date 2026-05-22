import { loadCreditWeights, loadRateLimits } from "./kvconfig.js";
import { buildUpstreamRequest, callUpstream, sanitizeChunk, sanitizeFinalUsage } from "./upstream.js";
import { classifyOperation, computeChargeCredits, estimateTokens, reserveCredits, refundReservation } from "./credits.js";
import { checkRpm, checkBurstBudget } from "./ratelimit.js";

// UUID v4 shape: 8-4-4-4-12 hex, 3rd group starts with '4', 4th group with [89ab]
const UUID_V4_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

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
  if (typeof body.request_id !== "string" || !UUID_V4_RE.test(body.request_id)) {
    return errResponse("BAD_REQUEST", "request_id must be a UUID v4", 400);
  }

  // Replay defense: if this request_id was previously used by this user,
  // reject — never re-stream a paid-for response.
  const replay = await env.CLAUGE_DB.prepare(
    "SELECT id FROM credit_usage_log WHERE user_id = ? AND request_id = ?"
  ).bind(userId, body.request_id).first();
  if (replay) {
    return errResponse("DUPLICATE_REQUEST", "request_id already used", 409);
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
    "SELECT credit_allowance_per_cycle, plan FROM users WHERE user_id = ?"
  )
    .bind(userId)
    .first();
  if (!userRow || userRow.plan !== "pro") {
    return errResponse("NOT_PRO", "Clauge AI requires Pro", 403);
  }

  if (!env.AI_UPSTREAM_MODEL) {
    return errResponse("AI_UNAVAILABLE", "Clauge AI is not configured", 503);
  }

  // Atomic CAS reservation. Two concurrent requests at the same near-zero
  // balance can't both pass — only one will get the credits, the other
  // gets 402. The reservation is settled in `finally` (debit any delta,
  // or refund overage if the upstream cost ended up below estimate).
  const reserveAmount = Math.max(estCharge, weights.min_credits_per_call ?? 1);
  if (!(await reserveCredits(userId, reserveAmount, env))) {
    return errResponse("INSUFFICIENT_CREDITS", "out of Clauge AI credits this cycle", 402);
  }

  if (!(await checkBurstBudget(userId, userRow.credit_allowance_per_cycle, limits.burst_budget_fraction, limits.burst_window_seconds, estCharge, env))) {
    await refundReservation(userId, reserveAmount, env);
    return errResponse("BURST_LIMITED", "using credits too quickly, try in an hour", 429);
  }

  const upReq = buildUpstreamRequest({
    messages: body.messages,
    model: env.AI_UPSTREAM_MODEL,
    systemSuffix: typeof body.system === "string" ? body.system : "",
    tools: Array.isArray(body.tools) ? body.tools : undefined,
    reasoningEffort: typeof body.reasoning_effort === "string" ? body.reasoning_effort : undefined,
  });
  let upResp;
  try {
    upResp = await callUpstream(upReq, env);
  } catch {
    await refundReservation(userId, reserveAmount, env);
    return errResponse("AI_BUSY", "Clauge AI is busy, retry in a moment", 503);
  }
  if (!upResp.ok || !upResp.body) {
    await refundReservation(userId, reserveAmount, env);
    return errResponse("AI_BUSY", "Clauge AI is busy, retry in a moment", 503);
  }

  const { readable, writable } = new TransformStream();
  const writer = writable.getWriter();
  const dec = new TextDecoder();
  const enc = new TextEncoder();
  let finalUsage = { prompt_tokens: estTokens, completion_tokens: 0, cost_usd_micros: 0 };
  let outputBytes = 0;
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
            // Don't forward upstream's [DONE] yet — the `finally` block
            // emits `event: balance` and our own [DONE] AFTER credit
            // settlement so the client sees `balance` before terminating
            // its SSE parser on `[DONE]`.
            continue;
          }
          try {
            const obj = JSON.parse(dataStr);
            if (obj?.usage) finalUsage = sanitizeFinalUsage(obj);
            const clean = sanitizeChunk(obj);
            const chunkLine = "data: " + JSON.stringify(clean) + "\n\n";
            const chunkBytes = enc.encode(chunkLine);
            outputBytes += chunkBytes.length;
            await writer.write(chunkBytes);
          } catch {
            // Skip malformed frame silently
          }
        }
      }
    } catch {
      // Stream interrupted — client will retry. Still settle credits.
    } finally {
      // 1. Compute upstream cost-based charge.
      let charge = computeChargeCredits(operation, estTokens, finalUsage.cost_usd_micros, weights);

      // 2. If we never received a usage chunk, fall back to estimating from
      //    received output bytes. Never settle at 0 — that would let a stream
      //    that cut off right before the usage chunk land for free.
      if (finalUsage.cost_usd_micros === 0 && outputBytes > 0) {
        const estOutTokens = Math.ceil(outputBytes / 4);
        const fallbackCharge = computeChargeCredits(
          operation,
          estTokens + estOutTokens,
          0,
          weights
        );
        charge = Math.max(charge, fallbackCharge);
      }

      // 3. Settle against the reservation. delta>0 = need to debit more;
      //    delta<0 = over-reserved, refund the difference.
      const delta = charge - reserveAmount;
      if (delta > 0) {
        // Try to debit additional. If balance < delta (rare — only if a
        // refund happened concurrently), cap to what's there.
        const balRow = await env.CLAUGE_DB.prepare(
          "SELECT credits_remaining FROM users WHERE user_id = ?"
        ).bind(userId).first();
        const cappedDelta = Math.min(delta, balRow?.credits_remaining ?? 0);
        if (cappedDelta > 0) {
          await env.CLAUGE_DB.prepare(
            `UPDATE users SET
               credits_remaining = credits_remaining - ?,
               updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND credits_remaining >= ?`
          ).bind(cappedDelta, userId, cappedDelta).run();
        }
        charge = reserveAmount + cappedDelta;
      } else if (delta < 0) {
        await refundReservation(userId, -delta, env);
      }

      // 4. Log the settled usage. UNIQUE on (user_id, request_id) so
      //    a hypothetical replay race that slipped past the SELECT-replay
      //    check at request entry doesn't double-log.
      //    `mode` is sent by the desktop's stream_openai when provider === clauge.
      const mode = typeof body.mode === "string" ? body.mode : null;
      await env.CLAUGE_DB.prepare(
        `INSERT OR IGNORE INTO credit_usage_log
           (user_id, operation, clauge_credits, cost_usd_micros, request_id, mode)
         VALUES (?, ?, ?, ?, ?, ?)`
      ).bind(userId, operation, charge, finalUsage.cost_usd_micros, body.request_id, mode).run();

      // 5. Emit live balance event so the client updates without polling.
      //    MUST be written BEFORE [DONE]: SSE clients (incl. our Rust
      //    `stream_openai`) break their parser loop on `data: [DONE]`,
      //    so anything after that terminator is dropped on the floor.
      const after = await env.CLAUGE_DB.prepare(
        "SELECT credits_remaining FROM users WHERE user_id = ?"
      ).bind(userId).first();
      try {
        await writer.write(enc.encode(
          `event: balance\ndata: ${JSON.stringify({ remaining: after?.credits_remaining ?? 0 })}\n\n`
        ));
        await writer.write(enc.encode("data: [DONE]\n\n"));
      } catch {
        // writer may already be closed if stream errored — swallow
      }

      await writer.close();
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
        `SELECT occurred_at, operation, clauge_credits, cost_usd_micros, request_id, mode
           FROM credit_usage_log
          WHERE user_id = ? AND occurred_at < ?
          ORDER BY occurred_at DESC LIMIT ?`
      ).bind(userId, before, limit)
    : env.CLAUGE_DB.prepare(
        `SELECT occurred_at, operation, clauge_credits, cost_usd_micros, request_id, mode
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
