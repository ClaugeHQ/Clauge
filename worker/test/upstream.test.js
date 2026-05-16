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
});
