// FCM HTTP v1 push relay. The desktop posts here with the user's
// registered device tokens; we mint a Google OAuth2 access token from
// the FCM_SERVICE_ACCOUNT secret and fan the notification out per token.
// No DB tables — stateless apart from the in-isolate token cache.

import { json, err } from './cors.js';

const FCM_SCOPE = 'https://www.googleapis.com/auth/firebase.messaging';
const TOKEN_URL = 'https://oauth2.googleapis.com/token';
const REFRESH_MARGIN_MS = 5 * 60 * 1000; // refresh when <5min left

let tokenCache = null; // { accessToken, expiresAt: ms, clientEmail }

function b64urlEncode(bytes) {
  let bin = '';
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function b64urlEncodeString(s) {
  return b64urlEncode(new TextEncoder().encode(s));
}

// Service-account private_key is PEM (PKCS#8). Strip armor, decode the
// base64 body, and import as pkcs8 for RS256 signing.
async function importPrivateKey(pem) {
  const body = pem
    .replace(/-----BEGIN PRIVATE KEY-----/, '')
    .replace(/-----END PRIVATE KEY-----/, '')
    .replace(/\s+/g, '');
  const bin = atob(body);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return crypto.subtle.importKey(
    'pkcs8',
    bytes.buffer,
    { name: 'RSASSA-PKCS1-v1_5', hash: { name: 'SHA-256' } },
    false,
    ['sign'],
  );
}

async function mintAccessToken(account) {
  const now = Math.floor(Date.now() / 1000);
  const header = b64urlEncodeString(JSON.stringify({ alg: 'RS256', typ: 'JWT' }));
  const claims = b64urlEncodeString(JSON.stringify({
    iss: account.client_email,
    scope: FCM_SCOPE,
    aud: TOKEN_URL,
    iat: now,
    exp: now + 3600,
  }));
  const signingInput = `${header}.${claims}`;
  const key = await importPrivateKey(account.private_key);
  const sig = await crypto.subtle.sign(
    'RSASSA-PKCS1-v1_5',
    key,
    new TextEncoder().encode(signingInput),
  );
  const assertion = `${signingInput}.${b64urlEncode(new Uint8Array(sig))}`;

  const resp = await fetch(TOKEN_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'urn:ietf:params:oauth:grant-type:jwt-bearer',
      assertion,
    }),
  });
  if (!resp.ok) {
    throw new Error(`FCM token grant failed: ${resp.status}`);
  }
  const body = await resp.json();
  return {
    accessToken: body.access_token,
    expiresAt: Date.now() + (body.expires_in ?? 3600) * 1000,
    clientEmail: account.client_email,
  };
}

async function getAccessToken(account) {
  if (
    tokenCache &&
    tokenCache.clientEmail === account.client_email &&
    Date.now() < tokenCache.expiresAt - REFRESH_MARGIN_MS
  ) {
    return tokenCache.accessToken;
  }
  tokenCache = await mintAccessToken(account);
  return tokenCache.accessToken;
}

function isShortString(v, max) {
  return typeof v === 'string' && v.length > 0 && v.length <= max;
}

function validateBody(body) {
  if (!body || typeof body !== 'object') return 'invalid body';
  const { fcmTokens, title, data } = body;
  if (!Array.isArray(fcmTokens) || fcmTokens.length < 1 || fcmTokens.length > 10) {
    return 'fcmTokens must be an array of 1..10 tokens';
  }
  if (!fcmTokens.every((t) => isShortString(t, 4096))) {
    return 'fcmTokens entries must be strings <= 4096 chars';
  }
  if (!isShortString(title, 256)) return 'title must be a string <= 256 chars';
  if (!isShortString(body.body, 256)) return 'body must be a string <= 256 chars';
  if (data !== undefined) {
    if (!data || typeof data !== 'object' || Array.isArray(data)) {
      return 'data must be an object';
    }
    for (const v of Object.values(data)) {
      if (typeof v !== 'string' || v.length > 256) {
        return 'data values must be strings <= 256 chars';
      }
    }
  }
  return null;
}

async function sendOne(projectId, accessToken, token, title, notifBody, data) {
  const resp = await fetch(
    `https://fcm.googleapis.com/v1/projects/${projectId}/messages:send`,
    {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${accessToken}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        message: {
          token,
          notification: { title, body: notifBody },
          ...(data ? { data } : {}),
          android: { priority: 'high' },
        },
      }),
    },
  );
  if (resp.ok) return { ok: true };

  let errorCode = `http_${resp.status}`;
  try {
    const errBody = await resp.json();
    const status = errBody?.error?.status;
    const fcmCode = errBody?.error?.details?.find(
      (d) => typeof d?.errorCode === 'string',
    )?.errorCode;
    errorCode = fcmCode || status || errorCode;
  } catch { /* keep http_* code */ }

  const stale = errorCode === 'UNREGISTERED' || errorCode === 'NOT_FOUND';
  return stale ? { ok: false, error: errorCode, stale: true } : { ok: false, error: errorCode };
}

// POST /api/push/send — bearer-authed (userId resolved by the router).
export async function handlePushSend(request, env, userId) {
  if (!env.FCM_SERVICE_ACCOUNT) return err(env, 503, 'push_not_configured');

  let account;
  try {
    account = JSON.parse(env.FCM_SERVICE_ACCOUNT);
  } catch {
    return err(env, 503, 'push_not_configured');
  }
  if (!account?.client_email || !account?.private_key || !account?.project_id) {
    return err(env, 503, 'push_not_configured');
  }

  let body;
  try {
    body = await request.json();
  } catch {
    return err(env, 400, 'invalid JSON');
  }
  const invalid = validateBody(body);
  if (invalid) return err(env, 400, invalid);

  let accessToken;
  try {
    accessToken = await getAccessToken(account);
  } catch (e) {
    console.error('FCM token mint failed:', e);
    return err(env, 502, 'push_token_grant_failed');
  }

  const results = await Promise.all(
    body.fcmTokens.map((token) =>
      sendOne(account.project_id, accessToken, token, body.title, body.body, body.data)
        .catch(() => ({ ok: false, error: 'send_failed' })),
    ),
  );

  return json(env, { results });
}
