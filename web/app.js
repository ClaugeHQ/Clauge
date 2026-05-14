/* Clauge docs — interactivity
   Hero typewriter · live demo cycling · AI conversation playback
   Header scroll · platform detect · reveal-on-scroll */

(() => {
  'use strict';

  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const $ = (sel, el = document) => el.querySelector(sel);
  const $$ = (sel, el = document) => Array.from(el.querySelectorAll(sel));

  /* ── header scroll state + hero preview retract ── */
  const header = $('#site-header');
  const heroPreview = $('.hero-preview');
  if (header || heroPreview) {
    const PREVIEW_RANGE = 240;
    const onScroll = () => {
      const y = window.scrollY;
      if (header) header.classList.toggle('scrolled', y > 8);
      if (heroPreview) {
        const p = Math.max(0, Math.min(1, 1 - y / PREVIEW_RANGE));
        heroPreview.style.setProperty('--preview-progress', p.toFixed(3));
      }
    };
    onScroll();
    window.addEventListener('scroll', onScroll, { passive: true });
  }

  /* ── platform detect (Apple Silicon vs Intel) ── */
  function detectArch() {
    let arch = 'Apple Silicon';
    try {
      const canvas = document.createElement('canvas');
      const gl = canvas.getContext('webgl');
      const ext = gl && gl.getExtension('WEBGL_debug_renderer_info');
      if (ext) {
        const renderer = (gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) || '').toLowerCase();
        if (renderer.includes('intel') && !renderer.includes('apple')) arch = 'Intel';
      }
    } catch { /* keep default */ }
    return arch;
  }
  const arch = detectArch();
  const archChip = $('#cta-arch');
  const dlArch = $('#dl-arch');
  if (archChip) archChip.textContent = arch;
  if (dlArch) dlArch.textContent = arch;
  // If we're confident this is an Apple Silicon Mac, the Intel link still stays — users may be looking for a build for another machine.

  /* ── hero typewriter rotator ── */
  const verbEl = $('#typeline-verb');
  if (verbEl) {
    const phrases = [
      'writes your SQL',
      'sends your API requests',
      'edits your code',
      'manages Claude Code sessions',
      'tunnels through SSH',
      'queries Mongo and Redis',
      'reviews your pull requests',
      'explains your stack traces',
      'runs your shell — and asks first'
    ];
    if (reduceMotion) {
      verbEl.textContent = 'powers your dev day';
    } else {
      let pi = 0, ci = 0, deleting = false;
      const tick = () => {
        const phrase = phrases[pi];
        if (!deleting) {
          ci++;
          verbEl.textContent = phrase.slice(0, ci);
          if (ci >= phrase.length) {
            deleting = true;
            return setTimeout(tick, 1700);
          }
          return setTimeout(tick, 36 + Math.random() * 38);
        } else {
          ci--;
          verbEl.textContent = phrase.slice(0, ci);
          if (ci <= 0) {
            deleting = false;
            pi = (pi + 1) % phrases.length;
            return setTimeout(tick, 240);
          }
          return setTimeout(tick, 18 + Math.random() * 22);
        }
      };
      setTimeout(tick, 700);
    }
  }

  /* ── live app demo + AI conversation ── */
  const stage = $('#app-stage');
  if (stage) {
    const railBtns = $$('.rail-btn', stage);
    const pills    = $$('.legend-pill', stage);
    const panels   = $$('.panel', stage);
    const aiSide   = $('#ai-side', stage);
    const aiStream = $('#ai-stream', stage);

    const ORDER = ['agent', 'rest', 'sql', 'nosql', 'ssh'];
    const ACCENT = {
      agent: 'var(--agent)',
      rest:  'var(--rest)',
      sql:   'var(--sql)',
      nosql: 'var(--nosql)',
      ssh:   'var(--ssh)',
    };

    /* per-mode status-bar text */
    const STATUS = {
      agent: { state: 'connected', context: '~/dev/clauge · auth-refactor', right: ['S:42%', 'W:18%', 'claude · sonnet 4.6'] },
      rest:  { state: 'staging',   context: 'acme · staging env',           right: ['201 · 84 ms', 'history · 142', 'auth · bearer'] },
      sql:   { state: 'connected', context: 'analytics_prod · ClickHouse',  right: ['5 rows', '318 ms', 'tunneled · bastion-eu'] },
      nosql: { state: 'connected', context: 'mongodb · prod',               right: ['users · 14,302', 'redis · cache', 'tls'] },
      ssh:   { state: 'connected', context: 'deploy@bastion-eu',            right: ['ed25519 · keychain', 'pty · 132×38', 'auto-run · off'] },
    };
    const sbMode = document.getElementById('sb-mode');
    const sbContext = document.getElementById('sb-context');
    const sbRightHost = document.querySelector('.status-bar .sb-right');

    const setStatus = (mode) => {
      const s = STATUS[mode];
      if (!s) return;
      if (sbMode) sbMode.innerHTML = `<span class="sb-dot on"></span><span>${s.state}</span>`;
      if (sbContext) sbContext.textContent = s.context;
      if (sbRightHost) sbRightHost.innerHTML = s.right.map(t => {
        const m = /^([A-Z]):/i.exec(t);
        return m
          ? `<span class="sb-chip"><b>${m[1]}</b>${t.slice(m[0].length - 1)}</span>`
          : `<span class="sb-chip">${t}</span>`;
      }).join('');
    };

    /* per-mode AI conversation script */
    const SCRIPTS = {
      agent: [
        { delay: 350,  type: 'user', html: 'Refactor the auth middleware to use the new session helper.' },
        { delay: 900,  type: 'ai',   html: 'Looking at <code>middleware.ts</code> and <code>session.ts</code>…' },
        { delay: 1100, type: 'tool', head: 'Edit', html: '<code>src/auth/middleware.ts</code> · <code>+24 / −9</code>' },
        { delay: 900,  type: 'ai',   html: '✓ Two files updated. Want me to run <code>bun check</code>?' },
      ],
      rest: [
        { delay: 300,  type: 'user', html: 'Test the payments endpoint with $24 USD.' },
        { delay: 850,  type: 'ai',   html: 'Building <code>POST /payments/intents</code> with your <code>staging</code> token.' },
        { delay: 1000, type: 'tool', head: 'Send request', html: '<code>amount: 2400, currency: "usd"</code>' },
        { delay: 850,  type: 'ai',   html: '<b>201 Created</b> · <code>pi_3Rt4z2K…</code> · 84&nbsp;ms.' },
      ],
      sql: [
        { delay: 300,  type: 'user', html: 'Top plans by active users this week.' },
        { delay: 900,  type: 'ai',   html: 'Reading the <code>events</code> schema in <code>analytics_prod</code>.' },
        { delay: 1050, type: 'tool', head: 'Run query', html: '<code>GROUP BY plan ORDER BY actives DESC</code>' },
        { delay: 900,  type: 'ai',   html: '<b>pro</b> 4,182 · <b>team</b> 1,517 · <b>free</b> 9,402.' },
      ],
      nosql: [
        { delay: 300,  type: 'user', html: 'Find pro users who logged in this week.' },
        { delay: 900,  type: 'ai',   html: 'Filtering <code>users</code> by <code>plan</code> and <code>last_seen</code>.' },
        { delay: 1050, type: 'tool', head: 'Run find', html: '<code>db.users.find({ plan: "pro", last_seen: { $gte: "2026-04-23" } })</code>' },
        { delay: 900,  type: 'ai',   html: '<b>4,182</b> documents. Want a histogram by signup date?' },
      ],
      ssh: [
        { delay: 300,  type: 'user', html: 'What\'s eating disk on the bastion?' },
        { delay: 900,  type: 'ai',   html: 'I\'ll propose a read-only command. You approve before it runs.' },
        { delay: 1050, type: 'tool', head: 'Confirm shell', html: '<code>du -sh /var/* | sort -h | tail -5</code> &nbsp;<span style="color:var(--text-faint);">[Cancel] [Run]</span>' },
        { delay: 900,  type: 'ai',   html: '<b>/var/log</b> · 42 GB · <b>/var/cache</b> · 18 GB.' },
      ],
    };

    let aiTimers = [];
    const clearAi = () => {
      aiTimers.forEach(t => clearTimeout(t));
      aiTimers = [];
      if (aiStream) aiStream.innerHTML = '';
    };

    const playScript = (mode) => {
      clearAi();
      if (!aiStream) return;
      const steps = SCRIPTS[mode] || [];
      let cum = 0;
      steps.forEach((step) => {
        cum += step.delay;
        aiTimers.push(setTimeout(() => {
          const b = document.createElement('div');
          b.className = 'bubble ' + step.type;
          if (step.type === 'tool') {
            b.innerHTML = `<div class="tool-head"><i class="fa-solid fa-screwdriver-wrench"></i> ${step.head}</div><div>${step.html}</div>`;
          } else {
            b.innerHTML = step.html;
          }
          aiStream.appendChild(b);
          // keep stream from overflowing — drop oldest if more than 5
          while (aiStream.children.length > 5) aiStream.removeChild(aiStream.firstChild);
        }, cum));
      });
    };

    const setMode = (mode) => {
      railBtns.forEach(b => {
        const on = b.dataset.mode === mode;
        b.classList.toggle('active', on);
        b.setAttribute('aria-selected', on ? 'true' : 'false');
      });
      pills.forEach(p => {
        const on = p.dataset.mode === mode;
        p.classList.toggle('active', on);
        p.setAttribute('aria-selected', on ? 'true' : 'false');
      });
      panels.forEach(p => p.classList.toggle('active', p.dataset.mode === mode));
      if (aiSide) aiSide.style.setProperty('--ai-accent', ACCENT[mode]);
      setStatus(mode);
      playScript(mode);
    };

    railBtns.forEach(b => b.addEventListener('click', () => { manualOverride(); setMode(b.dataset.mode); }));
    pills.forEach(p   => p.addEventListener('click', () => { manualOverride(); setMode(p.dataset.mode); }));

    /* auto-cycle */
    let cycleIndex = 0;
    let cycleTimer = null;
    let pausedUntil = 0;
    const CYCLE_MS = 7500;

    const tick = () => {
      if (Date.now() < pausedUntil) return;
      cycleIndex = (cycleIndex + 1) % ORDER.length;
      setMode(ORDER[cycleIndex]);
    };
    const startCycle = () => {
      if (reduceMotion || cycleTimer) return;
      cycleTimer = setInterval(tick, CYCLE_MS);
    };
    const stopCycle = () => {
      if (cycleTimer) { clearInterval(cycleTimer); cycleTimer = null; }
    };
    const manualOverride = () => {
      pausedUntil = Date.now() + 14000;
      const active = stage.querySelector('.panel.active');
      cycleIndex = ORDER.indexOf(active ? active.dataset.mode : 'agent');
    };

    stage.addEventListener('mouseenter', stopCycle);
    stage.addEventListener('mouseleave', startCycle);
    document.addEventListener('visibilitychange', () => {
      if (document.hidden) stopCycle(); else startCycle();
    });

    if ('IntersectionObserver' in window) {
      const io = new IntersectionObserver((entries) => {
        for (const e of entries) {
          if (e.isIntersecting) startCycle(); else stopCycle();
        }
      }, { threshold: 0.25 });
      io.observe(stage);
    } else {
      startCycle();
    }

    // kick off the first conversation immediately
    setMode('agent');
  }

  /* ── reveal on scroll ── */
  const reveals = $$('.reveal');
  if (reveals.length) {
    if (reduceMotion || !('IntersectionObserver' in window)) {
      reveals.forEach(el => el.classList.add('is-in'));
    } else {
      const io = new IntersectionObserver((entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            e.target.classList.add('is-in');
            io.unobserve(e.target);
          }
        }
      }, { threshold: 0.12, rootMargin: '0px 0px -40px 0px' });
      reveals.forEach(el => io.observe(el));
    }
  }
})();

/* ── Site footer — single source of truth, injected into <footer id="site-footer"> on every page ── */
(() => {
  const mount = document.getElementById('site-footer');
  if (!mount) return;

  const year = new Date().getFullYear();

  mount.innerHTML = `
    <div class="container">
      <div class="row">
        <div class="col" style="max-width: 320px;">
          <a class="brand" href="./" style="margin-bottom: 8px;">
            <img src="clauge-mark.svg" alt="" />
            <span>Clauge</span>
          </a>
          <p style="color: var(--text-dim);">The AI-powered super-app for developers.</p>
        </div>
        <div class="col">
          <h5>Product</h5>
          <a href="index.html#what-it-does">What it does</a>
          <a href="index.html#ai">The AI</a>
          <a href="index.html#themes">Themes</a>
          <a href="pricing.html">Pricing</a>
          <a href="changelog.html">Changelog</a>
        </div>
        <div class="col">
          <h5>Open</h5>
          <a href="https://github.com/ansxuman/Clauge" target="_blank" rel="noopener">GitHub</a>
          <a href="https://github.com/ansxuman/Clauge/issues" target="_blank" rel="noopener">Report an issue</a>
          <a href="https://github.com/ansxuman/Clauge/releases" target="_blank" rel="noopener">Releases</a>
          <a href="https://github.com/ansxuman/Clauge/blob/main/LICENSE" target="_blank" rel="noopener">License</a>
        </div>
        <div class="col">
          <h5>Legal</h5>
          <a href="terms.html">Terms of Service</a>
          <a href="privacy.html">Privacy Policy</a>
          <a href="enterprise.html">Enterprise</a>
          <a href="mailto:support@clauge.in">Commercial licensing</a>
        </div>
      </div>
      <div class="meta-row">
        <span>© ${year} Clauge</span>
        <span>Made for developers · macOS · Windows · Linux</span>
      </div>
    </div>
  `;
})();
