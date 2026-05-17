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

export function buildUpstreamRequest({ messages, model, systemSuffix, tools }) {
  const withSystem = [
    { role: "system", content: (systemSuffix ?? "") + "\n" + IDENTITY_PROMPT },
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
