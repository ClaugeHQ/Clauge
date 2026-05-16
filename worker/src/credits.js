// Credit deduction is the security-critical path. Per spec §10b:
//   - Pre-check + compare-and-swap deduction
//   - Per-(user_id, request_id) idempotency (DB UNIQUE constraint)
//   - DB CHECK (credits_remaining >= 0) is the last line of defense

export function classifyOperation(reqBody) {
  if (Array.isArray(reqBody?.tools) && reqBody.tools.length > 0) {
    return "tool_call_round";
  }
  return "chat";
}

// Returns the integer credits to charge.
// Combines per-operation base with per-message size and actual cost.
export function computeChargeCredits(operation, estimatedTokens, costUsdMicros, weights) {
  const op = weights.operations[operation] ?? weights.operations.chat;
  const threshold = op.long_ctx_threshold_tokens ?? Infinity;
  const multiplier = estimatedTokens >= threshold ? (op.long_ctx_multiplier ?? 1) : 1;
  const baseCharge = (op.base ?? 1) * multiplier;
  const costCharge = Math.ceil(
    (costUsdMicros / 1_000_000) / weights.cost_to_clauge_credit_divisor_usd
  );
  return Math.max(baseCharge, costCharge, weights.min_credits_per_call ?? 1);
}

export async function precheckBalance(userId, estimate, env) {
  const row = await env.CLAUGE_DB.prepare(
    "SELECT credits_remaining FROM users WHERE user_id = ?"
  )
    .bind(userId)
    .first();
  if (!row) return false;
  return row.credits_remaining >= estimate;
}

// Returns true on success (deducted or already-deducted-via-idempotency),
// false on insufficient balance. Never throws.
export async function deductCredits(userId, { operation, clauge_credits, cost_usd_micros, request_id }, env) {
  // Idempotency check: if this request_id was already deducted, return success.
  const existing = await env.CLAUGE_DB.prepare(
    "SELECT id FROM credit_usage_log WHERE user_id = ? AND request_id = ?"
  )
    .bind(userId, request_id)
    .first();
  if (existing) return true;

  // Compare-and-swap deduction
  const upd = await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       credits_remaining = credits_remaining - ?,
       updated_at = CURRENT_TIMESTAMP
     WHERE user_id = ? AND credits_remaining >= ?`
  )
    .bind(clauge_credits, userId, clauge_credits)
    .run();
  if ((upd.meta?.changes ?? 0) === 0) return false;

  // Append usage log row (UNIQUE on (user_id, request_id) so retry-races are safe)
  await env.CLAUGE_DB.prepare(
    `INSERT OR IGNORE INTO credit_usage_log
       (user_id, operation, clauge_credits, cost_usd_micros, request_id)
     VALUES (?, ?, ?, ?, ?)`
  )
    .bind(userId, operation, clauge_credits, cost_usd_micros, request_id)
    .run();

  return true;
}

// Rough estimate from message contents — used by precheck only.
// 1 token ~= 4 chars (English average). Good enough for budgeting.
export function estimateTokens(messages) {
  let chars = 0;
  for (const m of messages ?? []) {
    if (typeof m.content === "string") chars += m.content.length;
  }
  return Math.ceil(chars / 4);
}
