import { cloudflarePool } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    setupFiles: ["./test/setup.js"],
    pool: cloudflarePool,
    workers: {
      wrangler: { configPath: "./wrangler.toml" },
      miniflare: {
        d1Databases: ["CLAUGE_DB"],
        kvNamespaces: ["CLAUGE_KV"],
        bindings: {
          POLAR_WEBHOOK_SECRET: "test_webhook_secret",
          POLAR_API_KEY: "test_api_key",
          POLAR_PRICE_MONTHLY: "price_test_monthly",
          POLAR_PRICE_YEARLY: "price_test_yearly",
          POLAR_DISCOUNT_INTRO: "disc_test_intro",
          AI_UPSTREAM_API_KEY: "test_upstream_key",
          AI_UPSTREAM_URL: "https://upstream.test.invalid/chat/completions",
        },
      },
    },
  },
});
