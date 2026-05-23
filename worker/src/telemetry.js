// Telemetry ingest — append-only daily heartbeats.
//
// Design contract (see migration 0009 for full rationale):
//   • Insert-only into `telemetry_pings`. No UPDATE.
//   • Auth is OPTIONAL: header present → `user_id` set; absent → row
//     inserted with `user_id = NULL`. Same endpoint serves both cohorts.
//   • Counts arrive pre-bucketed as short strings ("1-10", "11-100",
//     "101-1k", "1k+"). We do NOT store raw integers — the bucketing
//     happens client-side so the wire format itself is privacy-friendly.
//   • Hard payload cap (8 KB) so a malicious / buggy client can't blow
//     up a row. Anything bigger gets 400.
//   • Per-device rate limit (2 / 24h). Catches both shutdown-flush
//     + scheduled-flush within a day and protects against retry loops.
//   • Validates allowlists for `os` / `arch` so we don't store junk
//     that would violate the CHECK constraints and fail the INSERT.

import { json, err } from './cors.js';
import { authenticate } from './auth.js';

const MAX_PAYLOAD_BYTES = 8 * 1024;
const ALLOWED_OS    = new Set(['macos', 'win', 'linux']);
const ALLOWED_ARCH  = new Set(['aarch64', 'x86_64']);
const ALLOWED_BUCKETS = new Set(['0', '1-10', '11-100', '101-1k', '1k+']);
const DEVICE_ID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// Rate-limit key TTL = 24h + slack. KV evicts on its own afterwards.
const RATE_TTL_SECONDS = 24 * 60 * 60 + 60;
const RATE_LIMIT_PER_24H = 2;

/**
 * POST /api/telemetry/heartbeat
 *
 * Body (JSON):
 *   {
 *     "device_id": "<uuid>",
 *     "app_version": "3.0.0-alpha.18",
 *     "os": "macos" | "win" | "linux",
 *     "os_version": "15",                  // optional, major only
 *     "arch": "aarch64" | "x86_64",
 *     "install_type": "dmg" | "msi" | …,   // optional
 *     "locale": "en-US",                    // optional, max 12 chars
 *     "theme": "dark" | "light" | "auto",  // optional
 *     "modes_active": "rest,sql,ssh",       // optional, comma list
 *     "features": { "rest.execute": "1-10", … },  // optional
 *     "errors":   { "err.ssh_timeout": "1-10" },  // optional
 *     "db_buckets": { "db.ssh_profiles": "11-100" } // optional
 *   }
 *
 * Always returns 204 No Content on accepted, 4xx for client errors,
 * 5xx only for genuine server failures. No response body — the client
 * doesn't need to read anything back, and the fewer bytes the better
 * for the fire-and-forget path.
 */
export async function handleTelemetryHeartbeat(request, env) {
  // Reject oversize before parsing — protects against malicious clients
  // sending megabyte payloads that would still validate token-wise.
  const lengthHeader = request.headers.get('Content-Length');
  if (lengthHeader && Number(lengthHeader) > MAX_PAYLOAD_BYTES) {
    return err(env, 413, 'payload too large');
  }

  let payload;
  try {
    payload = await request.json();
  } catch {
    return err(env, 400, 'invalid json');
  }
  if (!payload || typeof payload !== 'object') {
    return err(env, 400, 'invalid payload');
  }

  // ── Required fingerprint fields ────────────────────────────────────
  const deviceId = String(payload.device_id ?? '');
  if (!DEVICE_ID_RE.test(deviceId)) return err(env, 400, 'invalid device_id');

  const appVersion = String(payload.app_version ?? '').slice(0, 32);
  if (!appVersion) return err(env, 400, 'app_version required');

  const os = String(payload.os ?? '');
  if (!ALLOWED_OS.has(os)) return err(env, 400, 'invalid os');

  const arch = String(payload.arch ?? '');
  if (!ALLOWED_ARCH.has(arch)) return err(env, 400, 'invalid arch');

  // ── Optional fingerprint (capped lengths to keep payload sane) ─────
  const osVersion   = sanitiseShort(payload.os_version, 16);
  const locale      = sanitiseShort(payload.locale, 12);
  const theme       = sanitiseShort(payload.theme, 8);
  const modesActive = sanitiseShort(payload.modes_active, 64);

  // ── Bucketed maps — all values must be in the allowed bucket set ──
  const features   = sanitiseBucketMap(payload.features);
  const errors     = sanitiseBucketMap(payload.errors);
  const dbBuckets  = sanitiseBucketMap(payload.db_buckets);

  // ── Auth: optional. If the bearer is valid → row is attributed. ───
  // We deliberately ignore auth FAILURES (expired token, bad signature)
  // and just record the ping anonymously rather than rejecting it.
  // The app's flush path is fire-and-forget; an auth blip there
  // shouldn't cost us the telemetry signal.
  let userId = null;
  try {
    const ctx = await authenticate(request, env);
    if (ctx?.userId) userId = ctx.userId;
  } catch {
    // Treat as anonymous — see comment above.
  }
  const hasAccount = userId == null ? 0 : 1;

  // ── Per-device rate limit (KV, 24h window) ─────────────────────────
  const rlKey = `rl:tel:${deviceId}`;
  const current = Number((await env.CLAUGE_KV.get(rlKey)) ?? 0);
  if (current >= RATE_LIMIT_PER_24H) {
    // Silently drop with 204 — client doesn't need to know it was
    // rate-limited; it just keeps trying on its 24h cadence.
    return new Response(null, { status: 204 });
  }
  await env.CLAUGE_KV.put(rlKey, String(current + 1), {
    expirationTtl: RATE_TTL_SECONDS,
  });

  // ── Insert ────────────────────────────────────────────────────────
  try {
    await env.CLAUGE_DB.prepare(
      `INSERT INTO telemetry_pings (
         device_id, user_id, app_version, os, os_version, arch,
         locale, theme, has_account,
         modes_active, features, errors, db_buckets
       ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    )
      .bind(
        deviceId,
        userId,
        appVersion,
        os,
        osVersion,
        arch,
        locale,
        theme,
        hasAccount,
        modesActive,
        JSON.stringify(features),
        JSON.stringify(errors),
        JSON.stringify(dbBuckets),
      )
      .run();
  } catch (e) {
    console.error('[telemetry] insert failed:', e && e.stack ? e.stack : e);
    return err(env, 500, 'insert failed');
  }

  return new Response(null, { status: 204 });
}

// ── Helpers ─────────────────────────────────────────────────────────

function sanitiseShort(value, maxLen) {
  if (value == null) return null;
  const s = String(value).slice(0, maxLen);
  return s.length === 0 ? null : s;
}

// Returns a plain object containing only keys with allowed bucket values.
// Caps total keys at 32 to bound row size. Discards malformed entries
// silently — clients can be wrong; we just don't let them poison the row.
function sanitiseBucketMap(value) {
  if (value == null || typeof value !== 'object' || Array.isArray(value)) {
    return {};
  }
  const out = {};
  let count = 0;
  for (const [key, val] of Object.entries(value)) {
    if (count >= 32) break;
    // Keys: ASCII identifier-ish, max 48 chars. Anything else is dropped.
    if (typeof key !== 'string' || key.length === 0 || key.length > 48) continue;
    if (!/^[a-z][a-z0-9._-]*$/i.test(key)) continue;
    if (typeof val !== 'string') continue;
    if (!ALLOWED_BUCKETS.has(val)) continue;
    // The "0" bucket is the absence-of-key default — drop it if it slips
    // through so we don't bloat rows with no-op entries.
    if (val === '0') continue;
    out[key] = val;
    count++;
  }
  return out;
}
