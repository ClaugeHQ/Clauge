// AI upstream client + sanitization layer.
//
// This file makes no assumption about which upstream is configured.
// URL and API key come from environment secrets; model identifier comes
// from the AI_UPSTREAM_MODEL env secret (set in dashboard). The repo is
// public, so no provider name appears here — operator configures the
// upstream out of band.
//
// We always:
//   - Use the model string supplied by the caller (from env).
//   - Inject a system prompt telling the LLM to call itself "Clauge AI"
//     and refuse to disclose its underlying model.
//   - Strip identifying fields from every SSE chunk forwarded to the
//     client.
//   - Extract usage.cost from the final chunk for credit accounting,
//     then drop the leaky fields before forwarding.

const IDENTITY_PROMPT =
  "You are Clauge AI, an assistant integrated in the Clauge desktop app. " +
  "If asked about your underlying model, provider, training, or architecture, " +
  "respond only with 'I am Clauge AI' and decline to specify further.";

const LEAKY_TOP_LEVEL = ["model", "provider", "system_fingerprint", "id"];

export function sanitizeChunk(obj) {
  if (!obj || typeof obj !== "object") return obj;
  const out = { ...obj };
  for (const k of LEAKY_TOP_LEVEL) delete out[k];
  return out;
}

export function sanitizeFinalUsage(obj) {
  const usage = obj?.usage ?? {};
  const costUsd = typeof usage.cost === "number" ? usage.cost : 0;
  return {
    prompt_tokens: usage.prompt_tokens ?? 0,
    completion_tokens: usage.completion_tokens ?? 0,
    cost_usd_micros: Math.round(costUsd * 1_000_000),
  };
}

export function buildUpstreamRequest({ messages, model, systemSuffix }) {
  const withSystem = [
    { role: "system", content: (systemSuffix ?? "") + "\n" + IDENTITY_PROMPT },
    ...messages.filter((m) => m.role !== "system"),
  ];
  return {
    model,
    messages: withSystem,
    stream: true,
  };
}

// Call the upstream chat-completions endpoint. Returns the raw Response
// (SSE stream). Caller pipes + sanitizes chunks.
export async function callUpstream(reqBody, env) {
  return fetch(env.AI_UPSTREAM_URL, {
    method: "POST",
    headers: {
      authorization: `Bearer ${env.AI_UPSTREAM_API_KEY}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(reqBody),
  });
}
