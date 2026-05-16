import { env } from "cloudflare:test";
import fs from "node:fs";
import path from "node:path";

const migrations = [
  "migrations/0001_init.sql",
  "migrations/0002_pro_recurring.sql",
];

const root = path.resolve(__dirname, "..");
for (const m of migrations) {
  const sql = fs.readFileSync(path.join(root, m), "utf8");
  const statements = sql
    .split(/;\s*\n/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0 && !s.startsWith("--"));
  for (const stmt of statements) {
    await env.CLAUGE_DB.prepare(stmt).run();
  }
}

export async function seedUser(opts = {}) {
  const slug = opts.slug ?? `user_${Math.random().toString(36).slice(2, 8)}`;
  const res = await env.CLAUGE_DB.prepare(
    `INSERT INTO users (primary_email, display_name, slug, plan)
     VALUES (?, ?, ?, ?)
     RETURNING user_id`
  )
    .bind(
      opts.email ?? `${slug}@test.invalid`,
      opts.displayName ?? slug,
      slug,
      opts.plan ?? "free"
    )
    .first();
  return res.user_id;
}
