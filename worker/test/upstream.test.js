import { describe, it, expect } from "vitest";
import { sanitizeChunk, sanitizeFinalUsage, buildUpstreamRequest } from "../src/upstream.js";

describe("sanitizeChunk", () => {
  it("strips model + provider + id + fingerprint from a streaming chunk", () => {
    const input = {
      id: "chatcmpl-abc",
      model: "family-a/model-x",
      provider: "VendorY",
      system_fingerprint: "fp_xyz",
      choices: [{ delta: { content: "hello" } }],
    };
    const out = sanitizeChunk(input);
    expect(out.model).toBeUndefined();
    expect(out.provider).toBeUndefined();
    expect(out.system_fingerprint).toBeUndefined();
    expect(out.id).toBeUndefined();
    expect(out.choices[0].delta.content).toBe("hello");
  });

  it("leaves the chunk intact if no leaky fields present", () => {
    const input = { choices: [{ delta: { content: "ok" } }] };
    expect(sanitizeChunk(input)).toEqual(input);
  });
});

describe("sanitizeFinalUsage", () => {
  it("extracts cost in micros and drops model/provider identifiers", () => {
    const input = {
      model: "family-b/model-y",
      usage: {
        prompt_tokens: 100,
        completion_tokens: 200,
        cost: 0.000543,
      },
    };
    const out = sanitizeFinalUsage(input);
    expect(out.cost_usd_micros).toBe(543);
    expect(out.prompt_tokens).toBe(100);
    expect(out.completion_tokens).toBe(200);
    expect(JSON.stringify(out)).not.toContain("family-b");
  });

  it("treats missing cost as 0", () => {
    const out = sanitizeFinalUsage({ usage: {} });
    expect(out.cost_usd_micros).toBe(0);
  });
});

describe("buildUpstreamRequest", () => {
  it("uses provided model + injects identity prompt as first system message", () => {
    const r = buildUpstreamRequest({
      messages: [{ role: "user", content: "hi" }],
      model: "auto-router-name",
      systemSuffix: "Be concise.",
    });
    expect(r.model).toBe("auto-router-name");
    expect(r.stream).toBe(true);
    expect(r.messages[0].role).toBe("system");
    expect(r.messages[0].content).toContain("Clauge AI");
    expect(r.models).toBeUndefined();
  });

  // Regression guard: prior implementation dropped caller system messages
  // and only kept IDENTITY_PROMPT + systemSuffix. That neutered every
  // mode-specific behavior prompt for Pro users (SQL target_status,
  // REST tool routing, etc.). Caller system messages must be preserved.
  it("concatenates caller system messages into the merged system block", () => {
    const callerSystem = "You are a SQL assistant. Always call list_tables first.";
    const r = buildUpstreamRequest({
      messages: [
        { role: "system", content: callerSystem },
        { role: "user", content: "show tables" },
      ],
      model: "m",
    });
    expect(r.messages[0].role).toBe("system");
    expect(r.messages[0].content).toContain("Clauge AI"); // IDENTITY_PROMPT
    expect(r.messages[0].content).toContain(callerSystem); // caller prompt
    // No duplicate system messages further down.
    const systemCount = r.messages.filter((m) => m.role === "system").length;
    expect(systemCount).toBe(1);
    // Caller's user turn survives.
    expect(r.messages[r.messages.length - 1]).toEqual({ role: "user", content: "show tables" });
  });

  it("handles operator systemSuffix + caller system together (identity → suffix → caller)", () => {
    const r = buildUpstreamRequest({
      messages: [
        { role: "system", content: "MODE_PROMPT_X" },
        { role: "user", content: "go" },
      ],
      model: "m",
      systemSuffix: "OPERATOR_NOTE",
    });
    const content = r.messages[0].content;
    const idxIdentity = content.indexOf("Clauge AI");
    const idxSuffix = content.indexOf("OPERATOR_NOTE");
    const idxCaller = content.indexOf("MODE_PROMPT_X");
    expect(idxIdentity).toBeGreaterThanOrEqual(0);
    expect(idxSuffix).toBeGreaterThan(idxIdentity);
    expect(idxCaller).toBeGreaterThan(idxSuffix);
  });

  it("whitelists reasoning_effort and sets reasoning.effort when valid", () => {
    for (const effort of ["disabled", "low", "medium", "high"]) {
      const r = buildUpstreamRequest({
        messages: [{ role: "user", content: "q" }],
        model: "m",
        reasoningEffort: effort,
      });
      expect(r.reasoning).toEqual({ effort });
    }
  });

  it("ignores reasoning_effort when unknown or wrong type", () => {
    for (const bad of ["extreme", "", 5, null, undefined, { effort: "low" }]) {
      const r = buildUpstreamRequest({
        messages: [{ role: "user", content: "q" }],
        model: "m",
        reasoningEffort: bad,
      });
      expect(r.reasoning).toBeUndefined();
    }
  });

  it("forwards tools verbatim when provided", () => {
    const tools = [{ type: "function", function: { name: "ping", parameters: {} } }];
    const r = buildUpstreamRequest({
      messages: [{ role: "user", content: "q" }],
      model: "m",
      tools,
    });
    expect(r.tools).toEqual(tools);
  });

  it("omits tools field entirely when not provided", () => {
    const r = buildUpstreamRequest({
      messages: [{ role: "user", content: "q" }],
      model: "m",
    });
    expect(r.tools).toBeUndefined();
  });
});
