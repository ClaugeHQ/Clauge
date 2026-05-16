import { env } from "cloudflare:test";
import { applyD1Migrations } from "cloudflare:test";
import { inject } from "vitest";

const migrations = inject("D1_MIGRATIONS");
await applyD1Migrations(env.CLAUGE_DB, migrations);

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
