import path from "node:path";
import { fileURLToPath } from "node:url";
import { readD1Migrations } from "@cloudflare/vitest-pool-workers";

const root = path.resolve(fileURLToPath(import.meta.url), "../..");

export default async function ({ provide }) {
  const migrations = await readD1Migrations(path.join(root, "migrations"));
  provide("D1_MIGRATIONS", migrations);
}
