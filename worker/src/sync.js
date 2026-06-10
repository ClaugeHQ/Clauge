// Sync route handlers. All require an authenticated user (caller passes ctx).

import { json, err } from './cors.js';
import {
  isValidKind, getSyncState, getSyncBlob, conditionalUpsertSyncBlob, wipeSyncBlobs,
  getSyncHistory, getSyncHistoryBlob,
} from './db.js';

const MAX_PAYLOAD_BYTES = 900_000; // ~900 KB after gzip; D1 row limit is 1 MB.

/** GET /api/sync/state  → [{ kind, contentHash, updatedAt }] */
export async function handleSyncState(request, env, ctx) {
  const rows = await getSyncState(env, ctx.userId);
  return json(env, rows.map((r) => ({
    kind:        r.kind,
    contentHash: r.content_hash,
    updatedAt:   r.updated_at,
    deviceId:    r.device_id ?? null,
    deviceName:  r.device_name ?? null,
  })));
}

/** GET /api/sync/pull/:kind  → { contentHash, updatedAt, payload }  (payload = base64 of gzip) */
export async function handleSyncPull(request, env, ctx, kind) {
  if (!isValidKind(kind)) return err(env, 400, 'Invalid kind');

  const row = await getSyncBlob(env, ctx.userId, kind);
  if (!row) return err(env, 404, 'No blob for this kind');

  // payload is BLOB (Uint8Array) — encode as base64 for JSON transport.
  const payloadBytes = row.payload instanceof Uint8Array
    ? row.payload
    : new Uint8Array(row.payload);

  return json(env, {
    kind:        row.kind,
    contentHash: row.content_hash,
    updatedAt:   row.updated_at,
    payload:     bytesToBase64(payloadBytes),
  });
}

/**
 * PUT /api/sync/push/:kind
 *
 * body: { contentHash, payload, prevHash? }
 *   - prevHash omitted     → first push for this kind (row must not exist)
 *   - prevHash '*'         → force overwrite (post-conflict "Keep my changes")
 *   - prevHash <hash>      → only succeed if remote still has that hash
 *
 * 200 → { kind, contentHash, updatedAt }
 * 412 → { error:'precondition_failed', currentHash, currentUpdatedAt }
 *         Caller decides to pull or to retry with prevHash:'*'.
 */
export async function handleSyncPush(request, env, ctx, kind) {
  if (!isValidKind(kind)) return err(env, 400, 'Invalid kind');

  const body = await safeJson(request);
  if (!body || !body.contentHash || !body.payload) {
    return err(env, 400, 'Missing contentHash or payload');
  }
  if (typeof body.contentHash !== 'string' || body.contentHash.length > 128) {
    return err(env, 400, 'Bad contentHash');
  }

  const prevHash = body.prevHash ?? null;
  if (
    prevHash !== null &&
    prevHash !== '*' &&
    (typeof prevHash !== 'string' || prevHash.length > 128)
  ) {
    return err(env, 400, 'Bad prevHash');
  }

  const deviceId   = typeof body.deviceId   === 'string' ? body.deviceId.slice(0, 64)   : null;
  const deviceName = typeof body.deviceName === 'string' ? body.deviceName.slice(0, 64) : null;

  let bytes;
  try {
    bytes = base64ToBytes(body.payload);
  } catch {
    return err(env, 400, 'payload must be base64');
  }
  if (bytes.byteLength > MAX_PAYLOAD_BYTES) {
    return err(env, 413, `Payload too large (${bytes.byteLength} bytes; max ${MAX_PAYLOAD_BYTES})`);
  }

  const result = await conditionalUpsertSyncBlob(
    env, ctx.userId, kind, prevHash, body.contentHash, bytes, deviceId, deviceName,
  );

  if (!result.updated) {
    return json(env, {
      error:            'precondition_failed',
      message:          'Remote has changed since this device last synced.',
      currentHash:      result.row?.content_hash || null,
      currentUpdatedAt: result.row?.updated_at   || null,
    }, 412);
  }

  return json(env, {
    kind,
    contentHash: result.row.content_hash,
    updatedAt:   result.row.updated_at,
  });
}

/** GET /api/sync/history/:kind  → [{ contentHash, deviceName, replacedAt }] (newest first, max 5) */
export async function handleSyncHistory(request, env, ctx, kind) {
  if (!isValidKind(kind)) return err(env, 400, 'Invalid kind');

  const rows = await getSyncHistory(env, ctx.userId, kind);
  return json(env, rows.map((r) => ({
    contentHash: r.content_hash,
    deviceName:  r.device_name ?? null,
    replacedAt:  r.replaced_at,
  })));
}

/** GET /api/sync/history/:kind/:hash  → { payload, contentHash }  (payload = base64 of gzip) */
export async function handleSyncHistoryBlob(request, env, ctx, kind, hash) {
  if (!isValidKind(kind)) return err(env, 400, 'Invalid kind');
  if (typeof hash !== 'string' || !hash || hash.length > 128) {
    return err(env, 400, 'Bad hash');
  }

  const row = await getSyncHistoryBlob(env, ctx.userId, kind, hash);
  if (!row) return err(env, 404, 'No history entry for this hash');

  const payloadBytes = row.payload instanceof Uint8Array
    ? row.payload
    : new Uint8Array(row.payload);

  return json(env, {
    payload:     bytesToBase64(payloadBytes),
    contentHash: row.content_hash,
  });
}

/** DELETE /api/sync/wipe  Header: X-Confirm: yes  → 200 */
export async function handleSyncWipe(request, env, ctx) {
  const confirm = (request.headers.get('X-Confirm') || '').toLowerCase();
  if (confirm !== 'yes') return err(env, 400, "X-Confirm: yes header required");
  await wipeSyncBlobs(env, ctx.userId);
  return json(env, { ok: true });
}

// ─── helpers ───────────────────────────────────────────────────────

function bytesToBase64(bytes) {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function base64ToBytes(b64) {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

async function safeJson(request) {
  try { return await request.json(); } catch { return null; }
}
