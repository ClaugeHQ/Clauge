// Clauge auth + sync Worker — entrypoint and router.

import { preflight, err } from './cors.js';
import {
  handleGitHubExchange, handleGoogleExchange, handleGoogleRefresh,
  handleMe, handleUpdateProfile, handleDeleteAccount, handleLink, handleUnlink,
  handleLegacyAuthToken, authenticate,
} from './auth.js';
import {
  handleSyncState, handleSyncPull, handleSyncPush, handleSyncWipe,
} from './sync.js';
import { handleBillingWebhook, handleCreateCheckout, handleCreatePortal } from './billing.js';
import { sweepPastDue } from './cron.js';
import { handleAiChat, handleAiBalance, handleAiUsage } from './ai.js';

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const path = url.pathname;
    const method = request.method;

    // CORS preflight — short-circuit before any routing.
    if (method === 'OPTIONS') return preflight(env);

    try {
      // ─── Legacy GitHub OAuth (one-release back-compat) ─────
      if (path === '/auth/token' && method === 'POST') {
        return await handleLegacyAuthToken(request, env);
      }

      // ─── /api/auth/* — no bearer required (token exchanges) ─
      if (path === '/api/auth/github/exchange' && method === 'POST') {
        return await handleGitHubExchange(request, env);
      }
      if (path === '/api/auth/google/exchange' && method === 'POST') {
        return await handleGoogleExchange(request, env);
      }
      if (path === '/api/auth/google/refresh' && method === 'POST') {
        return await handleGoogleRefresh(request, env);
      }

      // ─── /api/auth/me — bearer required ────────────────────
      if (path === '/api/auth/me' && method === 'GET') {
        return await handleMe(request, env);
      }
      if (path === '/api/auth/me' && method === 'PATCH') {
        return await handleUpdateProfile(request, env);
      }
      if (path === '/api/auth/me' && method === 'DELETE') {
        return await handleDeleteAccount(request, env);
      }

      // ─── Linking ───────────────────────────────────────────
      if (path === '/api/auth/link' && method === 'POST') {
        return await handleLink(request, env);
      }
      if (path === '/api/auth/unlink' && method === 'POST') {
        return await handleUnlink(request, env);
      }

      // ─── /api/ai/chat — bearer required ────────────────────
      if (request.method === 'POST' && path === '/api/ai/chat') {
        const ctx = await authenticate(request, env);
        return handleAiChat(request, env, ctx?.userId ?? null);
      }

      // ─── /api/ai/balance — bearer required ─────────────────
      if (request.method === 'GET' && path === '/api/ai/balance') {
        const ctx = await authenticate(request, env);
        return handleAiBalance(env, ctx?.userId ?? null);
      }

      // ─── /api/ai/usage — bearer required ───────────────────
      if (request.method === 'GET' && path === '/api/ai/usage') {
        const ctx = await authenticate(request, env);
        return handleAiUsage(env, ctx?.userId ?? null, new URL(request.url));
      }

      // ─── /api/billing/webhook — server-to-server, no bearer ─
      if (path === '/api/billing/webhook' && method === 'POST') {
        return await handleBillingWebhook(request, env);
      }

      // ─── /api/billing/checkout — bearer required ────────────
      if (path === '/api/billing/checkout' && method === 'POST') {
        const checkoutCtx = await authenticate(request, env);
        return handleCreateCheckout(request, env, checkoutCtx?.userId ?? null);
      }

      // ─── /api/billing/portal — bearer required ──────────────
      if (request.method === 'POST' && path === '/api/billing/portal') {
        const ctx = await authenticate(request, env);
        return handleCreatePortal(env, ctx?.userId ?? null);
      }

      // ─── /api/sync/* — bearer required ─────────────────────
      const syncCtx = await authenticate(request, env);

      if (path === '/api/sync/state' && method === 'GET') {
        if (!syncCtx) return err(env, 401, 'Not authenticated');
        return await handleSyncState(request, env, syncCtx);
      }

      // /api/sync/pull/:kind
      const pullMatch = path.match(/^\/api\/sync\/pull\/([a-z]+)$/);
      if (pullMatch && method === 'GET') {
        if (!syncCtx) return err(env, 401, 'Not authenticated');
        return await handleSyncPull(request, env, syncCtx, pullMatch[1]);
      }

      // /api/sync/push/:kind
      const pushMatch = path.match(/^\/api\/sync\/push\/([a-z]+)$/);
      if (pushMatch && method === 'PUT') {
        if (!syncCtx) return err(env, 401, 'Not authenticated');
        return await handleSyncPush(request, env, syncCtx, pushMatch[1]);
      }

      if (path === '/api/sync/wipe' && method === 'DELETE') {
        if (!syncCtx) return err(env, 401, 'Not authenticated');
        return await handleSyncWipe(request, env, syncCtx);
      }

      // Unknown route on a path the worker was matched for.
      return err(env, 404, 'Not found');
    } catch (e) {
      console.error('Worker exception:', e && e.stack ? e.stack : e);
      return err(env, 500, 'Internal error');
    }
  },
  async scheduled(event, env, ctx) {
    ctx.waitUntil(sweepPastDue(env));
  },
};
