// Clauge auth + sync Worker — entrypoint and router.

import { preflight, err } from './cors.js';
import {
  handleGitHubExchange, handleGoogleExchange, handleGoogleRefresh,
  handleMe, handleUpdateProfile, handleDeleteAccount, handleLink, handleUnlink,
  handleLegacyAuthToken, authenticate, buildMeResponse,
} from './auth.js';
import {
  handleSyncState, handleSyncPull, handleSyncPush, handleSyncWipe,
  handleSyncHistory, handleSyncHistoryBlob,
} from './sync.js';
import { handleBillingWebhook, handleCreateCheckout, handleCreatePortal, handleGetPricing } from './billing.js';
import { sweepPastDue } from './cron.js';
import { handleAiChat, handleAiBalance, handleAiUsage } from './ai.js';
import { handleTelemetryHeartbeat } from './telemetry.js';
import { checkKeyRpm } from './ratelimit.js';

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
        const ctx = await authenticate(request, env);
        if (!ctx?.userId) return new Response("unauthorized", { status: 401 });
        if (!(await checkKeyRpm(`me:${ctx.userId}`, 60, env))) {
          return new Response("rate limited", { status: 429 });
        }
        return buildMeResponse(env, ctx.userId);
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
        const ip = request.headers.get("cf-connecting-ip") ?? "unknown";
        if (!(await checkKeyRpm(`webhook:${ip}`, 100, env))) {
          return new Response("rate limited", { status: 429 });
        }
        return await handleBillingWebhook(request, env);
      }

      // ─── /api/billing/checkout — bearer required ────────────
      if (path === '/api/billing/checkout' && method === 'POST') {
        const checkoutCtx = await authenticate(request, env);
        const checkoutUserId = checkoutCtx?.userId ?? null;
        if (checkoutUserId && !(await checkKeyRpm(`checkout:${checkoutUserId}`, 5, env))) {
          return new Response("rate limited", { status: 429 });
        }
        return handleCreateCheckout(request, env, checkoutUserId);
      }

      // ─── /api/billing/pricing — public, no bearer ───────────
      if (request.method === 'GET' && path === '/api/billing/pricing') {
        return handleGetPricing(env);
      }

      // ─── /api/billing/portal — bearer required ──────────────
      if (request.method === 'POST' && path === '/api/billing/portal') {
        const ctx = await authenticate(request, env);
        const portalUserId = ctx?.userId ?? null;
        if (portalUserId && !(await checkKeyRpm(`portal:${portalUserId}`, 5, env))) {
          return new Response("rate limited", { status: 429 });
        }
        return handleCreatePortal(env, portalUserId);
      }

      // ─── /api/telemetry/heartbeat — auth OPTIONAL ──────────
      // Same endpoint for anonymous (no header) and logged-in
      // (bearer present) cohorts. Handler does its own auth + rate
      // limiting; we do NOT pre-authenticate here because that would
      // reject anonymous pings before they reach the handler.
      if (path === '/api/telemetry/heartbeat' && method === 'POST') {
        return await handleTelemetryHeartbeat(request, env);
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

      // /api/sync/history/:kind/:hash  (more specific — must match first)
      const histBlobMatch = path.match(/^\/api\/sync\/history\/([a-z_]+)\/([A-Za-z0-9_-]+)$/);
      if (histBlobMatch && method === 'GET') {
        if (!syncCtx) return err(env, 401, 'Not authenticated');
        return await handleSyncHistoryBlob(request, env, syncCtx, histBlobMatch[1], histBlobMatch[2]);
      }

      // /api/sync/history/:kind
      const histMatch = path.match(/^\/api\/sync\/history\/([a-z_]+)$/);
      if (histMatch && method === 'GET') {
        if (!syncCtx) return err(env, 401, 'Not authenticated');
        return await handleSyncHistory(request, env, syncCtx, histMatch[1]);
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
