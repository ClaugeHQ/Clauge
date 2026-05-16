// Runtime configuration loaders for the AI proxy. All operator-tunable
// values live in KV — no provider-specific strings (URLs, model names,
// model families) appear in this file or anywhere else in source.
//
// `loadUpstreamPool` returns an EMPTY placeholder when the KV key is
// missing. Callers check `pool.model` and refuse to call the upstream
// if unset. This forces an operator to populate KV before any AI
// request can succeed.

const EMPTY_POOL = { model: "", allow: [] };

const DEFAULT_WEIGHTS = {
  operations: {
    chat: { base: 1, long_ctx_threshold_tokens: 8000, long_ctx_multiplier: 2 },
    tool_call_round: { base: 3 },
    code_completion: { base: 1 },
  },
  cost_to_clauge_credit_divisor_usd: 0.01,
  min_credits_per_call: 1,
};

const DEFAULT_RATE_LIMITS = {
  per_user_rpm: 30,
  burst_budget_fraction: 0.10,
  burst_window_seconds: 3600,
};

async function loadJson(env, key, fallback) {
  const raw = await env.CLAUGE_KV.get(key);
  if (raw === null) return fallback;
  try {
    return JSON.parse(raw);
  } catch {
    return fallback;
  }
}

export async function loadUpstreamPool(env) {
  return loadJson(env, "ai:upstream_pool", EMPTY_POOL);
}

export async function loadCreditWeights(env) {
  return loadJson(env, "ai:credit_weights", DEFAULT_WEIGHTS);
}

export async function loadRateLimits(env) {
  return loadJson(env, "ai:rate_limits", DEFAULT_RATE_LIMITS);
}
