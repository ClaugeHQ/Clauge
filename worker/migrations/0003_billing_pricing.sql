-- Pro v2 — operator-tunable display pricing + optional per-plan discount.
-- Both tables are read by GET /api/billing/pricing (anonymous, edge-cached).
-- Pure additive migration. Run once via `wrangler d1 execute clauge-db --file=...`

-- Prices in USD, one row per plan. Operator updates via D1 console.
CREATE TABLE IF NOT EXISTS billing_pricing (
  plan_id    TEXT PRIMARY KEY CHECK (plan_id IN ('monthly','yearly')),
  price_usd  INTEGER NOT NULL CHECK (price_usd > 0)
);

-- At most one active discount per plan. Empty table = no discounts active.
-- code is nullable (auto-apply discounts attach via checkout-link URL instead).
CREATE TABLE IF NOT EXISTS billing_discount (
  plan_id  TEXT PRIMARY KEY CHECK (plan_id IN ('monthly','yearly')),
  percent  INTEGER NOT NULL CHECK (percent > 0 AND percent < 100),
  code     TEXT
);

-- Seed default prices so the endpoint returns useful data immediately.
-- Operator overrides via: UPDATE billing_pricing SET price_usd = X WHERE plan_id = '...';
INSERT OR IGNORE INTO billing_pricing (plan_id, price_usd) VALUES
  ('monthly', 15),
  ('yearly',  150);
