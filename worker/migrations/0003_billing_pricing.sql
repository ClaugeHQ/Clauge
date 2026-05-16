-- Pro v2 — operator-tunable display pricing + optional discount
-- Pure additive migration. Run once via `wrangler d1 execute clauge-db --file=...`

CREATE TABLE IF NOT EXISTS billing_pricing (
  plan_id    TEXT PRIMARY KEY CHECK (plan_id IN ('monthly','yearly')),
  label      TEXT NOT NULL,
  price_usd  INTEGER NOT NULL CHECK (price_usd > 0),
  period     TEXT NOT NULL CHECK (period IN ('month','year'))
);

CREATE TABLE IF NOT EXISTS billing_discount (
  id       INTEGER PRIMARY KEY CHECK (id = 1),
  percent  INTEGER NOT NULL CHECK (percent > 0 AND percent < 100),
  label    TEXT NOT NULL,
  code     TEXT
);

-- Seed default prices. Operator edits via D1 console: UPDATE billing_pricing SET price_usd = X WHERE plan_id = '...';
INSERT OR IGNORE INTO billing_pricing (plan_id, label, price_usd, period) VALUES
  ('monthly', 'Monthly', 15,  'month'),
  ('yearly',  'Yearly',  150, 'year');

-- billing_discount starts empty (no active discount).
-- To start a discount: INSERT OR REPLACE INTO billing_discount VALUES (1, 25, 'Holiday sale', 'XMAS25');
-- To end a discount:   DELETE FROM billing_discount;
