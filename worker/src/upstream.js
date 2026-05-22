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

// Tight identity prompt. Two things to get right:
//   1. The model must respond normally and helpfully to ordinary queries
//      (greetings, questions, tool calls) — NOT lead every reply with a
//      disclaimer. Earlier wording ("respond only with 'I am Clauge AI'")
//      was interpreted as a default template; the model parroted it on
//      every message.
//   2. ONLY when the user asks about the underlying model / provider /
//      training / architecture should it deflect briefly.
const IDENTITY_PROMPT =
  "You are Clauge AI, the assistant built into the Clauge desktop app. " +
  "Respond normally and helpfully to all questions, greetings, and tool requests — " +
  "do not start replies with disclaimers. " +
  "Only if the user explicitly asks about your underlying model, provider, " +
  "training data, or architecture, reply briefly with \"I'm Clauge AI\" and decline " +
  "to elaborate. For every other topic, answer as you normally would.";

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

export function buildUpstreamRequest({ messages, model, systemSuffix, tools, reasoningEffort }) {
  // The desktop encodes mode-specific behavior (SQL target_status branching,
  // REST tool routing, ask-vs-act intent rules, etc.) in messages[] with
  // role="system". Earlier this function dropped them and only kept
  // IDENTITY_PROMPT + systemSuffix — which neutered every mode prompt for
  // Pro users. Now we concatenate caller system messages so the upstream
  // sees: IDENTITY_PROMPT → operator suffix (KV-tunable) → caller's prompt.
  // Order matters: identity first so the model's persona is set before any
  // mode rules; caller's prompt last so the most specific guidance is what
  // the model sees right before the user turn.
  const callerSystem = (messages ?? [])
    .filter((m) => m.role === "system")
    .map((m) => (typeof m.content === "string" ? m.content : ""))
    .filter(Boolean)
    .join("\n\n");
  const combinedSystem = [IDENTITY_PROMPT, systemSuffix ?? "", callerSystem]
    .map((s) => (typeof s === "string" ? s.trim() : ""))
    .filter(Boolean)
    .join("\n\n");
  const withSystem = [
    { role: "system", content: combinedSystem },
    ...messages.filter((m) => m.role !== "system"),
  ];
  const req = {
    model,
    messages: withSystem,
    stream: true,
  };
  // Forward the tools array verbatim when the caller provides one — the
  // upstream is OpenAI-compatible and accepts `tools: [{ type: "function",
  // function: {...} }]`. We do not inspect or rewrite the tools so the
  // desktop's full mode-specific tool set (REST / SQL / NoSQL / SSH /
  // Explorer) works end-to-end without per-tool worker awareness.
  if (Array.isArray(tools) && tools.length > 0) {
    req.tools = tools;
  }
  // Reasoning depth. Upstreams that support a `reasoning` field (Hy3 has
  // disabled/low/high) use this to balance speed/cost vs depth. Unknown
  // upstreams ignore the field. Allowed values are whitelisted to keep
  // junk from the caller (or a stale client) from leaking through.
  if (typeof reasoningEffort === "string"
      && ["disabled", "low", "medium", "high"].includes(reasoningEffort)) {
    req.reasoning = { effort: reasoningEffort };
  }
  return req;
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
