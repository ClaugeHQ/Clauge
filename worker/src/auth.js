// Auth route handlers + the authenticate() middleware used by all /api/* endpoints.

import { json, err } from './cors.js';
import {
  upsertUserWithIdentity, linkProviderToUser, unlinkProvider,
  getUserById, getProvidersForUser, deleteUser,
} from './db.js';
import { getCachedGitHubUser, setCachedGitHubUser } from './kv.js';
import { verifyGoogleIdToken } from './jwks.js';

// ─── Provider exchanges ────────────────────────────────────────────

/** POST /api/auth/github/exchange  body: { code }  → { token, refresh?, user, providers, plan } */
export async function handleGitHubExchange(request, env) {
  const body = await safeJson(request);
  if (!body || !body.code) return err(env, 400, 'Missing code');

  const tokenResp = await fetch('https://github.com/login/oauth/access_token', {
    method: 'POST',
    headers: {
      'Accept':       'application/json',
      'Content-Type': 'application/json',
      'User-Agent':   'Clauge-Worker',
    },
    body: JSON.stringify({
      client_id:     env.GITHUB_CLIENT_ID,
      client_secret: env.GITHUB_CLIENT_SECRET,
      code:          body.code,
    }),
  });
  const tokenData = await tokenResp.json().catch(() => ({}));
  if (tokenData.error || !tokenData.access_token) {
    return err(env, 400, tokenData.error_description || tokenData.error || 'GitHub token exchange failed');
  }

  const accessToken = tokenData.access_token;
  const profile = await fetchGitHubProfile(accessToken);
  if (!profile) return err(env, 502, 'Could not fetch GitHub profile');

  const { userId } = await upsertUserWithIdentity(env, 'github', profile.id, {
    email:          profile.email,
    displayName:    profile.name || profile.login,
    firstName:      splitName(profile.name).first,
    lastName:       splitName(profile.name).last,
    avatarUrl:      profile.avatar_url,
    providerLogin:  profile.login,
    slugSeed:       profile.login,
  });

  // Warm the KV cache so the very next /auth/me call doesn't redo the GitHub lookup.
  await setCachedGitHubUser(env, accessToken, {
    userId,
    githubLogin: profile.login,
    githubId:    profile.id,
    cachedAt:    Date.now(),
  });

  return await buildAuthSuccess(env, userId, { token: accessToken });
}

/** POST /api/auth/google/exchange  body: { code, redirectUri }  → { token, refresh, user, providers, plan } */
export async function handleGoogleExchange(request, env) {
  const body = await safeJson(request);
  if (!body || !body.code) return err(env, 400, 'Missing code');
  const redirectUri = body.redirectUri || 'https://clauge.in/auth/google-callback.html';

  const params = new URLSearchParams({
    code:          body.code,
    client_id:     env.GOOGLE_CLIENT_ID,
    client_secret: env.GOOGLE_CLIENT_SECRET,
    redirect_uri:  redirectUri,
    grant_type:    'authorization_code',
  });

  const tokenResp = await fetch('https://oauth2.googleapis.com/token', {
    method:  'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body:    params.toString(),
  });
  const tokenData = await tokenResp.json().catch(() => ({}));
  if (!tokenResp.ok || !tokenData.id_token) {
    return err(env, 400, tokenData.error_description || tokenData.error || 'Google token exchange failed');
  }

  let claims;
  try {
    claims = await verifyGoogleIdToken(tokenData.id_token, env.GOOGLE_CLIENT_ID);
  } catch (e) {
    return err(env, 401, 'id_token verification failed: ' + (e.message || ''));
  }

  const sub        = claims.sub;
  const email      = claims.email;
  const given      = claims.given_name  || null;
  const family     = claims.family_name || null;
  const fullName   = claims.name || [given, family].filter(Boolean).join(' ').trim() || email;
  const avatar     = claims.picture || null;
  const slugSeed   = (email && email.split('@')[0]) || (given || '').toLowerCase();

  const { userId } = await upsertUserWithIdentity(env, 'google', sub, {
    email,
    displayName:    fullName,
    firstName:      given,
    lastName:       family,
    avatarUrl:      avatar,
    providerLogin:  email,
    slugSeed,
  });

  return await buildAuthSuccess(env, userId, {
    token:   tokenData.access_token,
    refresh: tokenData.refresh_token || null,
    idToken: tokenData.id_token,
    expiresIn: tokenData.expires_in || null,
  });
}

/** POST /api/auth/google/refresh  body: { refreshToken }  → { token, idToken, expiresIn } */
export async function handleGoogleRefresh(request, env) {
  const body = await safeJson(request);
  if (!body || !body.refreshToken) return err(env, 400, 'Missing refreshToken');

  const params = new URLSearchParams({
    client_id:     env.GOOGLE_CLIENT_ID,
    client_secret: env.GOOGLE_CLIENT_SECRET,
    refresh_token: body.refreshToken,
    grant_type:    'refresh_token',
  });

  const resp = await fetch('https://oauth2.googleapis.com/token', {
    method:  'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body:    params.toString(),
  });
  const data = await resp.json().catch(() => ({}));
  if (!resp.ok || !data.access_token) {
    return err(env, 400, data.error_description || data.error || 'Google refresh failed');
  }
  return json(env, {
    token:     data.access_token,
    idToken:   data.id_token || null,
    expiresIn: data.expires_in || null,
  });
}

// ─── /api/auth/me & account management ─────────────────────────────

/** GET /api/auth/me  → { user, providers, plan, entitlements } */
export async function handleMe(request, env) {
  const ctx = await authenticate(request, env);
  if (!ctx) return err(env, 401, 'Not authenticated');
  return await buildMeResponse(env, ctx.userId);
}

/** PATCH /api/auth/me  body: { displayName?, firstName?, lastName? }  → fresh me response */
export async function handleUpdateProfile(request, env) {
  const ctx = await authenticate(request, env);
  if (!ctx) return err(env, 401, 'Not authenticated');
  const body = await safeJson(request);
  if (!body || typeof body !== 'object') return err(env, 400, 'Invalid body');

  const clean = (v, max) => {
    if (v === undefined || v === null) return undefined;
    if (typeof v !== 'string') return null; // explicit error marker
    const s = v.trim().slice(0, max);
    return s.length > 0 ? s : null; // empty string clears the field
  };
  const displayName = clean(body.displayName, 120);
  const firstName   = clean(body.firstName,   80);
  const lastName    = clean(body.lastName,    80);

  if (displayName === null && body.displayName !== '' && body.displayName !== null && body.displayName !== undefined) {
    return err(env, 400, 'displayName must be a string');
  }

  // Only update fields the client sent (undefined = don't touch).
  const sets = [];
  const binds = [];
  if (displayName !== undefined) { sets.push('display_name = ?'); binds.push(displayName); }
  if (firstName   !== undefined) { sets.push('first_name = ?');   binds.push(firstName); }
  if (lastName    !== undefined) { sets.push('last_name = ?');    binds.push(lastName); }
  if (sets.length === 0) return err(env, 400, 'No fields to update');

  sets.push('updated_at = CURRENT_TIMESTAMP');
  binds.push(ctx.userId);

  await env.CLAUGE_DB
    .prepare(`UPDATE users SET ${sets.join(', ')} WHERE user_id = ?`)
    .bind(...binds)
    .run();

  return await buildMeResponse(env, ctx.userId);
}

/** DELETE /api/auth/me  Headers: X-Confirm: <slug>  → 200 on success */
export async function handleDeleteAccount(request, env) {
  const ctx = await authenticate(request, env);
  if (!ctx) return err(env, 401, 'Not authenticated');
  const confirm = request.headers.get('X-Confirm') || '';

  const user = await getUserById(env, ctx.userId);
  if (!user) return err(env, 404, 'User not found');
  if (confirm !== user.slug) {
    return err(env, 400, 'X-Confirm header must match your slug');
  }

  await deleteUser(env, ctx.userId);
  return json(env, { ok: true });
}

/** POST /api/auth/link  body: { provider: 'github'|'google', code, redirectUri? }
 *  Links an additional provider to the CURRENT user (auth via bearer token). */
export async function handleLink(request, env) {
  const ctx = await authenticate(request, env);
  if (!ctx) return err(env, 401, 'Not authenticated');

  const body = await safeJson(request);
  if (!body || !body.code || !body.provider) return err(env, 400, 'Missing provider/code');

  if (body.provider === 'github') {
    const tokenResp = await fetch('https://github.com/login/oauth/access_token', {
      method: 'POST',
      headers: { 'Accept': 'application/json', 'Content-Type': 'application/json', 'User-Agent': 'Clauge-Worker' },
      body: JSON.stringify({
        client_id:     env.GITHUB_CLIENT_ID,
        client_secret: env.GITHUB_CLIENT_SECRET,
        code:          body.code,
      }),
    });
    const td = await tokenResp.json().catch(() => ({}));
    if (!td.access_token) return err(env, 400, td.error_description || 'GitHub token exchange failed');
    const p = await fetchGitHubProfile(td.access_token);
    if (!p) return err(env, 502, 'GitHub profile fetch failed');
    try {
      await linkProviderToUser(env, ctx.userId, 'github', p.id, {
        providerLogin: p.login,
        email:         p.email,
      });
    } catch (e) {
      if (e.code === 'IDENTITY_TAKEN') return err(env, 409, e.message);
      throw e;
    }
    return await buildMeResponse(env, ctx.userId);
  }

  if (body.provider === 'google') {
    const params = new URLSearchParams({
      code:          body.code,
      client_id:     env.GOOGLE_CLIENT_ID,
      client_secret: env.GOOGLE_CLIENT_SECRET,
      redirect_uri:  body.redirectUri || 'https://clauge.in/auth/google-callback.html',
      grant_type:    'authorization_code',
    });
    const r = await fetch('https://oauth2.googleapis.com/token', {
      method:  'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body:    params.toString(),
    });
    const td = await r.json().catch(() => ({}));
    if (!td.id_token) return err(env, 400, td.error_description || 'Google token exchange failed');
    let claims;
    try {
      claims = await verifyGoogleIdToken(td.id_token, env.GOOGLE_CLIENT_ID);
    } catch (e) {
      return err(env, 401, 'id_token verification failed');
    }
    try {
      await linkProviderToUser(env, ctx.userId, 'google', claims.sub, {
        providerLogin: claims.email,
        email:         claims.email,
      });
    } catch (e) {
      if (e.code === 'IDENTITY_TAKEN') return err(env, 409, e.message);
      throw e;
    }
    return await buildMeResponse(env, ctx.userId);
  }

  return err(env, 400, 'Unknown provider');
}

/** POST /api/auth/unlink  body: { provider }  → 200 / 409 if last provider */
export async function handleUnlink(request, env) {
  const ctx = await authenticate(request, env);
  if (!ctx) return err(env, 401, 'Not authenticated');

  const body = await safeJson(request);
  if (!body || !body.provider) return err(env, 400, 'Missing provider');

  const ok = await unlinkProvider(env, ctx.userId, body.provider);
  if (!ok) return err(env, 409, 'Cannot unlink your only sign-in method. Link another provider first.');
  return await buildMeResponse(env, ctx.userId);
}

// ─── Legacy /auth/token handler (kept for back-compat one release) ─

/** POST /auth/token  legacy GitHub-only exchange. Same shape as before; warns deprecated. */
export async function handleLegacyAuthToken(request, env) {
  const body = await safeJson(request);
  if (!body || !body.code) return err(env, 400, 'Missing code', { 'X-Deprecated': 'Use /api/auth/github/exchange' });

  const r = await fetch('https://github.com/login/oauth/access_token', {
    method: 'POST',
    headers: { 'Accept': 'application/json', 'Content-Type': 'application/json', 'User-Agent': 'Clauge-Worker' },
    body: JSON.stringify({
      client_id:     env.GITHUB_CLIENT_ID,
      client_secret: env.GITHUB_CLIENT_SECRET,
      code:          body.code,
    }),
  });
  const data = await r.json().catch(() => ({}));
  if (data.error) {
    return err(env, 400, data.error_description || data.error, { 'X-Deprecated': 'Use /api/auth/github/exchange' });
  }
  return json(env, { access_token: data.access_token }, 200, { 'X-Deprecated': 'Use /api/auth/github/exchange' });
}

// ─── Middleware: authenticate a request via bearer token + X-Provider ─

/**
 * Resolves the bearer token + X-Provider header to a user context.
 * Returns { userId } on success, null on failure (caller returns 401).
 */
export async function authenticate(request, env) {
  const authHeader = request.headers.get('Authorization') || '';
  const m = authHeader.match(/^Bearer\s+(.+)$/i);
  if (!m) return null;
  const token = m[1].trim();
  const provider = (request.headers.get('X-Provider') || '').toLowerCase();

  if (provider === 'github') {
    return await authenticateGitHub(env, token);
  }
  if (provider === 'google') {
    return await authenticateGoogle(env, token);
  }
  return null;
}

async function authenticateGitHub(env, token) {
  const cached = await getCachedGitHubUser(env, token);
  if (cached && cached.userId) return { userId: cached.userId };

  const profile = await fetchGitHubProfile(token);
  if (!profile) return null;

  // Resolve user_id from oauth_identities (must already exist from /exchange).
  const row = await env.CLAUGE_DB
    .prepare('SELECT user_id FROM oauth_identities WHERE provider = ? AND provider_user_id = ?')
    .bind('github', String(profile.id))
    .first();
  if (!row) return null;

  await setCachedGitHubUser(env, token, { userId: row.user_id, githubLogin: profile.login, githubId: profile.id, cachedAt: Date.now() });
  return { userId: row.user_id };
}

async function authenticateGoogle(env, token) {
  // For Google we expect an id_token (JWT). Verify offline.
  // (If the client passes an access_token by mistake, we'd need a tokeninfo
  // round-trip. We document id_token usage to keep the hot path offline.)
  try {
    const claims = await verifyGoogleIdToken(token, env.GOOGLE_CLIENT_ID);
    const row = await env.CLAUGE_DB
      .prepare('SELECT user_id FROM oauth_identities WHERE provider = ? AND provider_user_id = ?')
      .bind('google', claims.sub)
      .first();
    if (!row) return null;
    return { userId: row.user_id };
  } catch {
    return null;
  }
}

// ─── Shared helpers ────────────────────────────────────────────────

async function fetchGitHubProfile(token) {
  const r = await fetch('https://api.github.com/user', {
    headers: {
      'Authorization': `Bearer ${token}`,
      'Accept':        'application/json',
      'User-Agent':    'Clauge-Worker',
    },
  });
  if (!r.ok) return null;
  const profile = await r.json();
  // /user only returns the public email; fetch /user/emails for the primary verified one.
  if (!profile.email) {
    try {
      const er = await fetch('https://api.github.com/user/emails', {
        headers: { 'Authorization': `Bearer ${token}`, 'Accept': 'application/json', 'User-Agent': 'Clauge-Worker' },
      });
      if (er.ok) {
        const emails = await er.json();
        const primary = emails.find((e) => e.primary && e.verified) || emails.find((e) => e.verified);
        if (primary) profile.email = primary.email;
      }
    } catch { /* best effort */ }
  }
  return profile;
}

function splitName(fullName) {
  if (!fullName) return { first: null, last: null };
  const parts = fullName.trim().split(/\s+/);
  if (parts.length === 1) return { first: parts[0], last: null };
  return { first: parts[0], last: parts.slice(1).join(' ') };
}

async function buildAuthSuccess(env, userId, tokens) {
  const user = await getUserById(env, userId);
  const providers = await getProvidersForUser(env, userId);
  return json(env, {
    ...tokens,
    user:        serializeUser(user),
    providers:   providers.map(serializeProvider),
    plan:        user.plan,
    entitlements: entitlementsForPlan(user.plan),
  });
}

async function buildMeResponse(env, userId) {
  const user = await env.CLAUGE_DB.prepare(
    `SELECT user_id, primary_email, display_name, first_name, last_name, avatar_url, slug,
            plan, subscription_status, cancel_at_period_end,
            current_period_end, credit_allowance_per_cycle, credits_remaining
       FROM users WHERE user_id = ?`
  ).bind(userId).first();
  if (!user) return err(env, 404, 'User not found');

  const providers = await getProvidersForUser(env, userId);
  const ent = entitlementsForPlan(user.plan);
  ent.credits = {
    remaining:  user.credits_remaining,
    allowance:  user.credit_allowance_per_cycle,
    resets_at:  user.current_period_end,
  };
  ent.subscription = {
    status:              user.subscription_status,
    cancel_at_period_end: !!user.cancel_at_period_end,
  };

  return json(env, {
    user:         serializeUser(user),
    providers:    providers.map(serializeProvider),
    plan:         user.plan,
    entitlements: ent,
  });
}

function serializeUser(u) {
  return {
    userId:       u.user_id,
    email:        u.primary_email,
    displayName:  u.display_name,
    firstName:    u.first_name,
    lastName:     u.last_name,
    avatarUrl:    u.avatar_url,
    slug:         u.slug,
  };
}

function serializeProvider(p) {
  return {
    provider:       p.provider,
    providerUserId: p.provider_user_id,
    providerLogin:  p.provider_login,
    email:          p.email,
    linkedAt:       p.linked_at,
    lastSeenAt:     p.last_seen_at,
  };
}

function entitlementsForPlan(plan) {
  const isPro = plan === 'pro' || plan === 'enterprise';
  return {
    plan,
    features: {
      clauge_ai:           isPro,
      unlimited_coworkers: isPro,
      premium_themes:      isPro,
    },
  };
}

async function safeJson(request) {
  try {
    return await request.json();
  } catch {
    return null;
  }
}

export { entitlementsForPlan, buildMeResponse };
