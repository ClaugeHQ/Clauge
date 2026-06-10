// D1 helpers. All SQL goes through here so route handlers stay thin.

const RESERVED_SLUGS = new Set([
  'admin','api','auth','app','billing','clauge','docs','help','home','me',
  'new','org','orgs','pro','settings','signin','signout','signup',
  'support','team','teams','u','user','users','www',
]);

function slugify(input) {
  return (input || '')
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')        // strip diacritics
    .replace(/[^a-z0-9-]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40);
}

/** Generate a unique slug, falling back through candidates and then suffixing. */
async function generateUniqueSlug(env, candidates) {
  for (const cand of candidates) {
    const base = slugify(cand);
    if (!base || RESERVED_SLUGS.has(base)) continue;

    // Try the bare slug first, then -2, -3, ... up to -99.
    for (let n = 0; n < 100; n++) {
      const candidate = n === 0 ? base : `${base}-${n + 1}`;
      const row = await env.CLAUGE_DB
        .prepare('SELECT 1 FROM users WHERE LOWER(slug) = ? LIMIT 1')
        .bind(candidate)
        .first();
      if (!row) return candidate;
    }
  }
  // Last resort — deterministic random.
  return 'user-' + crypto.randomUUID().slice(0, 8);
}

/**
 * Upsert a user identified by (provider, providerUserId).
 *
 * profile: { email, displayName, firstName, lastName, avatarUrl, providerLogin, slugSeed }
 *
 * Returns { userId, isNew }.
 */
export async function upsertUserWithIdentity(env, provider, providerUserId, profile) {
  const existing = await env.CLAUGE_DB
    .prepare('SELECT user_id FROM oauth_identities WHERE provider = ? AND provider_user_id = ?')
    .bind(provider, String(providerUserId))
    .first();

  if (existing) {
    // Refresh last_seen + non-null profile fields on the users row.
    await env.CLAUGE_DB
      .prepare('UPDATE oauth_identities SET last_seen_at = CURRENT_TIMESTAMP, provider_login = ?, email = ? WHERE provider = ? AND provider_user_id = ?')
      .bind(profile.providerLogin || null, profile.email || null, provider, String(providerUserId))
      .run();
    await env.CLAUGE_DB
      .prepare(`UPDATE users
                   SET display_name = COALESCE(?, display_name),
                       first_name   = COALESCE(?, first_name),
                       last_name    = COALESCE(?, last_name),
                       avatar_url   = COALESCE(?, avatar_url),
                       primary_email= COALESCE(primary_email, ?),
                       updated_at   = CURRENT_TIMESTAMP
                 WHERE user_id = ?`)
      .bind(
        profile.displayName || null,
        profile.firstName   || null,
        profile.lastName    || null,
        profile.avatarUrl   || null,
        profile.email       || null,
        existing.user_id,
      )
      .run();
    return { userId: existing.user_id, isNew: false };
  }

  // Email-merge: if a Clauge user already exists with this verified email,
  // attach the new identity to that user instead of creating a duplicate
  // account. Both Google (id_token.email_verified) and GitHub (/user.email
  // primary verified) hand us verified emails, so this is safe.
  if (profile.email) {
    const byEmail = await env.CLAUGE_DB
      .prepare('SELECT user_id FROM users WHERE primary_email = ? COLLATE NOCASE')
      .bind(profile.email)
      .first();

    if (byEmail) {
      const targetUserId = byEmail.user_id;
      const dupProvider = await env.CLAUGE_DB
        .prepare('SELECT 1 FROM oauth_identities WHERE user_id = ? AND provider = ?')
        .bind(targetUserId, provider)
        .first();

      // Only merge if this user has no identity row for the same provider
      // yet (avoids the weird case where two providers of the same kind
      // somehow share an email but have different provider_user_ids).
      if (!dupProvider) {
        await env.CLAUGE_DB
          .prepare(`INSERT INTO oauth_identities (provider, provider_user_id, user_id, provider_login, email)
                    VALUES (?, ?, ?, ?, ?)`)
          .bind(provider, String(providerUserId), targetUserId, profile.providerLogin || null, profile.email || null)
          .run();
        await env.CLAUGE_DB
          .prepare(`UPDATE users
                       SET display_name = COALESCE(display_name, ?),
                           first_name   = COALESCE(first_name, ?),
                           last_name    = COALESCE(last_name, ?),
                           avatar_url   = COALESCE(avatar_url, ?),
                           updated_at   = CURRENT_TIMESTAMP
                     WHERE user_id = ?`)
          .bind(
            profile.displayName || null,
            profile.firstName   || null,
            profile.lastName    || null,
            profile.avatarUrl   || null,
            targetUserId,
          )
          .run();
        return { userId: targetUserId, isNew: false };
      }
    }
  }

  // New user — generate a slug from the provider's best handle, then create both rows.
  const slug = await generateUniqueSlug(env, [
    profile.slugSeed,
    profile.providerLogin,
    profile.email ? profile.email.split('@')[0] : null,
    'user',
  ].filter(Boolean));

  const insertUser = await env.CLAUGE_DB
    .prepare(`INSERT INTO users (primary_email, display_name, first_name, last_name, avatar_url, slug)
              VALUES (?, ?, ?, ?, ?, ?)
              RETURNING user_id`)
    .bind(
      profile.email       || null,
      profile.displayName || null,
      profile.firstName   || null,
      profile.lastName    || null,
      profile.avatarUrl   || null,
      slug,
    )
    .first();

  const userId = insertUser.user_id;

  await env.CLAUGE_DB
    .prepare(`INSERT INTO oauth_identities (provider, provider_user_id, user_id, provider_login, email)
              VALUES (?, ?, ?, ?, ?)`)
    .bind(provider, String(providerUserId), userId, profile.providerLogin || null, profile.email || null)
    .run();

  return { userId, isNew: true };
}

/** Link an additional provider to an EXISTING user. Throws if provider already linked to a different user. */
export async function linkProviderToUser(env, userId, provider, providerUserId, profile) {
  const existing = await env.CLAUGE_DB
    .prepare('SELECT user_id FROM oauth_identities WHERE provider = ? AND provider_user_id = ?')
    .bind(provider, String(providerUserId))
    .first();

  if (existing && existing.user_id !== userId) {
    const e = new Error('This provider account is already linked to a different Clauge user.');
    e.code = 'IDENTITY_TAKEN';
    throw e;
  }
  if (existing && existing.user_id === userId) {
    // Idempotent — touch last_seen and return.
    await env.CLAUGE_DB
      .prepare('UPDATE oauth_identities SET last_seen_at = CURRENT_TIMESTAMP WHERE provider = ? AND provider_user_id = ?')
      .bind(provider, String(providerUserId))
      .run();
    return;
  }

  await env.CLAUGE_DB
    .prepare(`INSERT INTO oauth_identities (provider, provider_user_id, user_id, provider_login, email)
              VALUES (?, ?, ?, ?, ?)`)
    .bind(provider, String(providerUserId), userId, profile.providerLogin || null, profile.email || null)
    .run();
}

/** Returns true if the unlink succeeded; false if the user only has this one provider. */
export async function unlinkProvider(env, userId, provider) {
  const count = await env.CLAUGE_DB
    .prepare('SELECT COUNT(*) AS n FROM oauth_identities WHERE user_id = ?')
    .bind(userId)
    .first();
  if (!count || count.n <= 1) return false;

  await env.CLAUGE_DB
    .prepare('DELETE FROM oauth_identities WHERE user_id = ? AND provider = ?')
    .bind(userId, provider)
    .run();
  return true;
}

export async function getUserById(env, userId) {
  return env.CLAUGE_DB
    .prepare(`SELECT user_id, primary_email, display_name, first_name, last_name,
                     avatar_url, slug, plan, plan_expires_at, polar_customer_id,
                     created_at, updated_at
              FROM users WHERE user_id = ?`)
    .bind(userId)
    .first();
}

export async function getProvidersForUser(env, userId) {
  const res = await env.CLAUGE_DB
    .prepare(`SELECT provider, provider_user_id, provider_login, email, linked_at, last_seen_at
              FROM oauth_identities WHERE user_id = ? ORDER BY linked_at`)
    .bind(userId)
    .all();
  return res.results || [];
}

export async function deleteUser(env, userId) {
  // FK cascade nukes oauth_identities + sync_blobs.
  await env.CLAUGE_DB.prepare('DELETE FROM users WHERE user_id = ?').bind(userId).run();
}

// ─── sync_blobs ────────────────────────────────────────────────────

export const SYNC_KINDS = ['rest', 'sql', 'nosql', 'agent', 'ssh', 'explorer', 'coworkers', 'workspace_notes', 'workspace_boards'];

export function isValidKind(kind) {
  return SYNC_KINDS.includes(kind);
}

export async function getSyncState(env, userId) {
  const res = await env.CLAUGE_DB
    .prepare('SELECT kind, content_hash, updated_at, device_id, device_name FROM sync_blobs WHERE user_id = ? ORDER BY kind')
    .bind(userId)
    .all();
  return res.results || [];
}

export async function getSyncBlob(env, userId, kind) {
  return env.CLAUGE_DB
    .prepare('SELECT kind, content_hash, updated_at, payload FROM sync_blobs WHERE user_id = ? AND kind = ?')
    .bind(userId, kind)
    .first();
}

/**
 * Conditional upsert (optimistic concurrency).
 *
 * `prevHash` semantics:
 *   - null / undefined  → require the row to NOT exist yet (first push).
 *                         A row that already exists with a different hash → 412.
 *   - '*'               → unconditional overwrite (used after the user picks
 *                         "Keep my changes" in the conflict resolver).
 *   - <hex string>      → only succeed if the existing row's content_hash
 *                         matches exactly. Mismatch → 412.
 *
 * Returns `{ updated, row }`. When `updated` is false, `row` carries the
 * remote's current state so the caller can surface it in the 412 body.
 */
export async function conditionalUpsertSyncBlob(env, userId, kind, prevHash, contentHash, payloadBytes, deviceId, deviceName) {
  const existing = await env.CLAUGE_DB
    .prepare('SELECT content_hash, updated_at FROM sync_blobs WHERE user_id = ? AND kind = ?')
    .bind(userId, kind)
    .first();

  // Mismatch detection — split by which "branch" of the precondition fails.
  if (existing && prevHash !== '*' && prevHash != null && prevHash !== existing.content_hash) {
    return { updated: false, row: existing };
  }
  if (!existing && prevHash != null && prevHash !== '*') {
    return { updated: false, row: null };
  }

  // Same-hash short-circuit — D1 write skipped, but we still report success
  // so the client's bookkeeping updates (no-op from the server's POV).
  if (existing && existing.content_hash === contentHash) {
    return { updated: true, row: existing };
  }

  const upsert = env.CLAUGE_DB
    .prepare(`INSERT INTO sync_blobs (user_id, kind, payload, content_hash, updated_at, device_id, device_name)
              VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP, ?, ?)
              ON CONFLICT(user_id, kind) DO UPDATE SET
                payload      = excluded.payload,
                content_hash = excluded.content_hash,
                updated_at   = excluded.updated_at,
                device_id    = excluded.device_id,
                device_name  = excluded.device_name`)
    .bind(userId, kind, payloadBytes, contentHash, deviceId ?? null, deviceName ?? null);

  if (existing) {
    // Replacing a live blob → archive the old row first, then prune the
    // history to the newest 5 versions. One atomic D1 batch.
    await env.CLAUGE_DB.batch([
      env.CLAUGE_DB
        .prepare(`INSERT INTO sync_blob_history (user_id, kind, payload, content_hash, device_name)
                  SELECT user_id, kind, payload, content_hash, device_name
                  FROM sync_blobs WHERE user_id = ?1 AND kind = ?2`)
        .bind(userId, kind),
      upsert,
      env.CLAUGE_DB
        .prepare(`DELETE FROM sync_blob_history
                  WHERE user_id = ?1 AND kind = ?2 AND id NOT IN (
                    SELECT id FROM sync_blob_history
                    WHERE user_id = ?1 AND kind = ?2
                    ORDER BY id DESC LIMIT 5)`)
        .bind(userId, kind),
    ]);
  } else {
    await upsert.run();
  }

  const fresh = await env.CLAUGE_DB
    .prepare('SELECT content_hash, updated_at FROM sync_blobs WHERE user_id = ? AND kind = ?')
    .bind(userId, kind)
    .first();
  return { updated: true, row: fresh };
}

export async function getSyncHistory(env, userId, kind) {
  const res = await env.CLAUGE_DB
    .prepare(`SELECT content_hash, device_name, replaced_at
              FROM sync_blob_history
              WHERE user_id = ? AND kind = ?
              ORDER BY id DESC LIMIT 5`)
    .bind(userId, kind)
    .all();
  return res.results || [];
}

export async function getSyncHistoryBlob(env, userId, kind, hash) {
  return env.CLAUGE_DB
    .prepare(`SELECT payload, content_hash
              FROM sync_blob_history
              WHERE user_id = ? AND kind = ? AND content_hash = ?
              ORDER BY id DESC LIMIT 1`)
    .bind(userId, kind, hash)
    .first();
}

export async function wipeSyncBlobs(env, userId) {
  await env.CLAUGE_DB.prepare('DELETE FROM sync_blobs WHERE user_id = ?').bind(userId).run();
}
