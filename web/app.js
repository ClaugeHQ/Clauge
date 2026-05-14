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

  /* ── live app demo: 8-mode rail, per-mode workspace + AI script ── */
  const stage = $('#app-stage');
  if (stage) {
    const railHost   = $('[data-rail]', stage);
    const tabbar     = $('[data-tabbar]', stage);
    const sideCtaLbl = $('[data-side-cta-label]', stage);
    const sideCards  = $('[data-side-cards]', stage);
    const sideList   = $('[data-side-list]', stage);
    const workspace  = $('[data-workspace]', stage);
    const aiPill     = $('[data-ai-pill]', stage);
    const aiStream   = $('[data-ai-stream]', stage);
    const aiInput    = $('[data-ai-input]', stage);
    const sbLeft     = $('[data-sb-left]', stage);
    const sbCenter   = $('[data-sb-center]', stage);

    const ORDER = ['agent', 'workspace', 'rest', 'sql', 'nosql', 'ssh', 'explorer', 'history'];

    /* Real Clauge sidebar SVGs (from src/lib/components/sidebar/Sidebar.svelte). */
    const RAIL_SVG = {
      agent:     '<svg viewBox="0 0 24 24"><path d="M12 3l1.6 4.8L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.2L12 3z"/><path d="M18.5 14l.9 2.6 2.6.9-2.6.9-.9 2.6-.9-2.6-2.6-.9 2.6-.9.9-2.6z"/></svg>',
      workspace: '<svg viewBox="0 0 24 24"><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg>',
      rest:      '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z"/></svg>',
      sql:       '<svg viewBox="0 0 24 24"><ellipse cx="12" cy="5" rx="8" ry="2.5"/><path d="M4 5v14c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5V5"/><path d="M4 12c0 1.4 3.6 2.5 8 2.5s8-1.1 8-2.5"/></svg>',
      nosql:     '<svg viewBox="0 0 24 24"><path d="M8 3a2 2 0 00-2 2v4a2 2 0 01-2 2H3a1 1 0 000 2h1a2 2 0 012 2v4a2 2 0 002 2"/><path d="M16 3a2 2 0 012 2v4a2 2 0 002 2h1a1 1 0 010 2h-1a2 2 0 00-2 2v4a2 2 0 01-2 2"/></svg>',
      ssh:       '<svg viewBox="0 0 24 24"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>',
      explorer:  '<svg viewBox="0 0 24 24"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>',
      history:   '<svg viewBox="0 0 24 24"><path d="M3 12a9 9 0 109-9 9.5 9.5 0 00-6.4 2.5L3 8"/><polyline points="3 3 3 8 8 8"/><polyline points="12 8 12 13 15 15"/></svg>',
    };

    const MODES = {
      agent: {
        label: 'Agent', pill: 'AGENT',
        sideCta: 'New Session',
        sideCards: [
          { icon: 'fa-folder-tree', label: 'Contexts', badge: '3' },
          { icon: 'fa-plug',        label: 'Plugins',  badge: '1' },
        ],
        sideListTitle: 'SESSIONS',
        sideListItems: [
          { glyph: 'A', name: 'atlas-payments',   meta: 'Code Review', badge: 'just now', active: true },
          { glyph: 'S', name: 'schema-migration', meta: 'Brainstorm',  badge: '2h' },
        ],
        aiPlaceholder: 'press up to edit queued · type to ask',
        sbLeft: '~/atlas-team/atlas / payments-refactor',
        sbCenter: '<span class="sb-chip"><b>S</b>:14%</span><span class="sb-chip"><b>W</b>:82%</span><span class="sb-chip">sonnet · 31%</span>',
        workspace: `
          <div class="ws-tabbar">
            <div class="ws-tab is-active"><span class="ws-tab-dot"></span>atlas-payments<i class="fa-solid fa-xmark"></i></div>
            <div class="ws-tab-add">+</div>
          </div>
          <div class="ws-scroll ag-pane">
            <div class="ag-head">
              <div class="ag-mark"></div>
              <div class="ag-meta">
                <div class="ag-title">Clauge Code <span class="muted">v2.1.128</span> · <span class="muted">Opus 4.7 (1M context)</span> · <span class="muted">Clauge Max</span></div>
                <div class="ag-path">~/atlas-team/atlas/.clauge-worktrees/payments-refactor</div>
              </div>
            </div>
            <div class="ag-user" data-agent-user></div>
            <div class="ag-stream" data-agent-stream></div>
            <div class="ag-queued">Add the analysis report in the workspace notes</div>
            <div class="ag-prompt">
              <span class="ag-caret">›</span>
              <input class="ag-input" data-ag-input type="text" placeholder="press up to edit queued · type to ask" />
              <span class="ag-hint">esc to interrupt · ⌘↵ to send</span>
            </div>
          </div>
        `,
        script: [
          { delay: 200,  type: 'user-type', target: '[data-agent-user]', text: 'Review the payments-refactor branch for bugs.' },
          { delay: 700,  type: 'agent', html: '<i class="fa-solid fa-circle-dot ok"></i> <b>Bash</b>(git log --oneline main..HEAD)<br/><span class="muted">L 3 commits · 9 files changed</span>' },
          { delay: 900,  type: 'ai',   html: 'Reading <code>payments/intent.ts</code>, <code>webhook.ts</code>, <code>refund.ts</code>...' },
          { delay: 1100, type: 'agent', html: '<i class="fa-solid fa-folder-open ok"></i> Read 3 files, called <span class="t-acc">plugin:context-mode</span> 2 times' },
          { delay: 1200, type: 'ai',   html: 'Found it — <code>confirmIntent()</code> returns the raw Stripe error string back to the client. That leaks gateway state.' },
          { delay: 1100, type: 'agent', html: '<i class="fa-solid fa-pen-to-square ok"></i> <b>Edit</b> <code>payments/intent.ts</code> · <span class="ok">+6</span> / <span class="bad">−2</span>' },
          { delay: 1000, type: 'thinking', html: 'Wrangling... <span class="muted">12s · ↑ 593 tokens</span>' },
          { delay: 900,  type: 'ai',   html: 'Patched. Open a Code Review note in Workspace?' },
        ],
      },

      workspace: {
        label: 'Workspace', pill: 'WORKSPACE',
        sideCta: 'New Workspace',
        sideCards: [
          { icon: 'fa-inbox',  label: 'Inbox',      badge: '2' },
          { icon: 'fa-robot',  label: 'Co-workers', badge: '4' },
        ],
        sideListTitle: 'WORKSPACES',
        sideListItems: [
          { glyph: 'A', name: 'atlas',         meta: '2 notes · 1 board', active: true },
          { glyph: 'D', name: 'design-system', meta: '5 notes' },
        ],
        aiPlaceholder: '',
        sbLeft: 'atlas / boards / Payments',
        sbCenter: '<span class="sb-chip">4 cards</span><span class="sb-chip">@alex</span>',
        workspace: `
          <div class="ws-tabbar">
            <div class="ws-tab is-active"><i class="fa-solid fa-table-cells"></i> Tasks<i class="fa-solid fa-xmark"></i></div>
            <div class="ws-tab"><i class="fa-solid fa-pen-to-square"></i> Code Review</div>
            <div class="ws-tab-add">+</div>
          </div>
          <div class="ws-scroll bd-pane">
            <div class="bd-banner"><i class="fa-solid fa-bolt"></i> Pull open issues from this project's Git remote into the board.</div>
            <div class="bd-cols">
              <div class="bd-col"><div class="bd-col-head">Backlog <span>0</span></div></div>
              <div class="bd-col bd-col-active">
                <div class="bd-col-head">Todo <span>4</span></div>
                <div class="bd-card"><div class="bd-card-title">Fix confirmIntent error leak</div><div class="bd-card-tags"><span class="pill pill-red mini">critical</span><span class="pill mini">payments</span></div><div class="bd-card-foot"><span class="bd-cw"><span class="bd-cw-avatar">A</span>@alex</span><span class="bd-card-time">2m</span></div></div>
                <div class="bd-card"><div class="bd-card-title">Refund webhook never acks</div><div class="bd-card-tags"><span class="pill pill-red mini">critical</span></div><div class="bd-card-foot"><span class="bd-cw"><span class="bd-cw-avatar">A</span>@alex</span><span class="bd-card-time">2m</span></div></div>
                <div class="bd-card" data-bd-assigning><div class="bd-card-title">Idempotency key cross-tenant</div><div class="bd-card-tags"><span class="pill pill-purple mini">race</span></div><div class="bd-card-foot"><span class="bd-cw bd-cw-empty" data-bd-cw>Assign…</span><span class="bd-card-time">5m</span></div></div>
                <div class="bd-card"><div class="bd-card-title">MainViewModel never shuts down</div><div class="bd-card-tags"><span class="pill mini">memory</span></div><div class="bd-card-foot"><span class="bd-cw"><span class="bd-cw-avatar">Q</span>@quinn</span><span class="bd-card-time">7m</span></div></div>
              </div>
              <div class="bd-col"><div class="bd-col-head">In Review <span>0</span></div></div>
              <div class="bd-col"><div class="bd-col-head">Done <span>0</span></div></div>
            </div>
          </div>
        `,
        script: [
          { delay: 500,  type: 'bd-pop' },
          { delay: 1200, type: 'bd-assign', text: 'A', name: '@alex' },
        ],
      },

      rest: {
        label: 'REST', pill: 'REST',
        sideCta: 'New Collection',
        sideCards: [
          { icon: 'fa-clock-rotate-left',      label: 'History',         badge: '142' },
          { icon: 'fa-arrow-right-arrow-left', label: 'Import / Export' },
        ],
        sideListTitle: 'COLLECTIONS',
        sideListItems: [
          { glyph: 'A', name: 'Atlas Smoke Tests',  meta: '6 requests', active: true },
          { glyph: 'A', name: 'Auth flows',         meta: '4 requests' },
          { glyph: 'P', name: 'Payments · staging', meta: '11 requests' },
        ],
        aiPlaceholder: 'e.g. POST create user with email and role',
        sbLeft: 'atlas · staging env',
        sbCenter: '<span class="sb-chip">5 / 6 passed</span><span class="sb-chip">avg 88 ms</span>',
        workspace: `
          <div class="ws-tabbar">
            <div class="ws-tab is-active"><i class="fa-solid fa-clock-rotate-left"></i> GET delayed-response<i class="fa-solid fa-xmark"></i></div>
            <div class="ws-tab-add">+</div>
          </div>
          <div class="ws-scroll rest-pane">
            <div class="url-row">
              <span class="method m-get">GET</span>
              <span class="url-input mono">https://api.atlas.dev/<span class="t-acc">{{env}}</span>/products?limit=10</span>
              <span class="url-send">Send <i class="fa-solid fa-paper-plane"></i></span>
            </div>
            <div class="rest-result"><div class="rest-rows" data-rest-rows></div></div>
          </div>
        `,
        script: [
          { delay: 300,  type: 'user-type', text: 'Run Atlas Smoke Tests in staging.' },
          { delay: 700,  type: 'ai', html: 'Executing 6 requests sequentially with the <code>staging</code> env...' },
          { delay: 600,  type: 'rest-row', html: '<span class="m-get pill-mini">GET</span> /health <span class="ok mono">200 · 41ms</span>' },
          { delay: 250,  type: 'rest-row', html: '<span class="m-post pill-mini">POST</span> /auth/token <span class="ok mono">200 · 187ms</span>' },
          { delay: 250,  type: 'rest-row', html: '<span class="m-get pill-mini">GET</span> /products?limit=10 <span class="ok mono">200 · 73ms</span>' },
          { delay: 250,  type: 'rest-row', html: '<span class="m-post pill-mini">POST</span> /cart/add <span class="ok mono">200 · 99ms</span>' },
          { delay: 250,  type: 'rest-row', html: '<span class="m-post pill-mini">POST</span> /checkout/intent <span class="bad mono">422 · 112ms</span>' },
          { delay: 250,  type: 'rest-row', html: '<span class="m-get pill-mini">GET</span> /webhooks/log <span class="ok mono">200 · 58ms</span>' },
          { delay: 600,  type: 'ai', html: '<b>5 passed · 1 failed</b> — <code>/checkout/intent</code> returned <code>intent_amount_too_low</code>. Retry with $24?' },
        ],
      },

      sql: {
        label: 'SQL', pill: 'SQL',
        sideCta: 'New Connection', sideCards: [],
        sideListTitle: 'CONNECTIONS',
        sideListItems: [
          { glyph: 'CH', name: 'atlas_events',   meta: 'ClickHouse · :8123' },
          { glyph: 'PG', name: 'atlas_prod',     meta: 'Postgres · :5432',   active: true },
          { glyph: 'PG', name: 'atlas_staging',  meta: 'Postgres · :5435' },
          { glyph: 'MY', name: 'reporting',      meta: 'MySQL · :3306' },
        ],
        aiPlaceholder: 'e.g. top 10 customers by revenue Q2',
        sbLeft: 'atlas_prod · Postgres',
        sbCenter: '<span class="sb-chip">5 rows</span><span class="sb-chip">126 ms</span>',
        workspace: `
          <div class="ws-tabbar">
            <div class="ws-tab is-active"><i class="fa-solid fa-database"></i> atlas_prod<i class="fa-solid fa-xmark"></i></div>
            <div class="ws-tab-add">+</div>
            <span class="ws-tab-env">Execute ▶</span>
          </div>
          <div class="ws-scroll sql-pane">
            <pre class="sql-editor"><code><span class="ln">1</span><span class="t-key">SELECT</span> name, <span class="t-fn">SUM</span>(total) <span class="t-key">AS</span> revenue
<span class="ln">2</span><span class="t-key">FROM</span>   customers c <span class="t-key">JOIN</span> orders o <span class="t-key">ON</span> o.customer_id = c.id
<span class="ln">3</span><span class="t-key">WHERE</span>  o.placed_at &gt;= <span class="t-str">'2026-04-01'</span>
<span class="ln">4</span><span class="t-key">GROUP BY</span> name <span class="t-key">ORDER BY</span> revenue <span class="t-key">DESC LIMIT</span> <span class="t-num">5</span>;</code></pre>
            <table class="sql-table"><thead><tr><th>name</th><th class="ral">revenue</th></tr></thead><tbody data-sql-rows></tbody></table>
          </div>
        `,
        script: [
          { delay: 300, type: 'user-type', text: 'Top 5 customers by revenue this quarter.' },
          { delay: 700, type: 'ai', html: 'Reading <code>atlas_prod</code> schema...' },
          { delay: 500, type: 'sql-row', html: '<td>Northwind Logistics</td><td class="ral mono">$48,210</td>' },
          { delay: 220, type: 'sql-row', html: '<td>Acme Provisions</td><td class="ral mono">$37,944</td>' },
          { delay: 220, type: 'sql-row', html: '<td>Boréal Coffee Co.</td><td class="ral mono">$29,118</td>' },
          { delay: 220, type: 'sql-row', html: '<td>Helix Robotics</td><td class="ral mono">$22,705</td>' },
          { delay: 220, type: 'sql-row', html: '<td>Yamashita Press</td><td class="ral mono">$18,302</td>' },
          { delay: 500, type: 'ai', html: 'Top — <b>Northwind Logistics</b> at <b>$48,210</b>.' },
        ],
      },

      nosql: {
        label: 'NoSQL', pill: 'NOSQL',
        sideCta: 'New Connection', sideCards: [],
        sideListTitle: 'CONNECTIONS',
        sideListItems: [
          { glyph: 'M', name: 'atlas',       meta: 'MongoDB · :27017', active: true },
          { glyph: 'R', name: 'atlas-cache', meta: 'Redis · :6379' },
        ],
        aiPlaceholder: 'e.g. find pro users inactive 30 days',
        sbLeft: 'atlas · MongoDB',
        sbCenter: '<span class="sb-chip">219 docs</span>',
        workspace: `
          <div class="ws-tabbar"><div class="ws-tab is-active"><i class="fa-solid fa-code"></i> users · find<i class="fa-solid fa-xmark"></i></div><span class="ws-tab-env">Run ▶</span></div>
          <div class="ws-scroll nosql-pane">
            <pre class="sql-editor"><code><span class="ln">1</span>db.<span class="t-fn">users</span>.<span class="t-fn">find</span>({
<span class="ln">2</span>  <span class="t-key">plan</span>: <span class="t-str">"pro"</span>,
<span class="ln">3</span>  <span class="t-key">last_seen</span>: { $lt: <span class="t-fn">ISODate</span>(<span class="t-str">"2026-04-14"</span>) }
<span class="ln">4</span>})</code></pre>
            <div class="json-list" data-nosql-rows></div>
          </div>
        `,
        script: [
          { delay: 300, type: 'user-type', text: 'Find Pro users inactive for 30 days.' },
          { delay: 700, type: 'ai', html: 'Filtering <code>users</code>...' },
          { delay: 500, type: 'json-row', html: '{ <b>email</b>: "r.bauer@northwind.co", <b>plan</b>: "pro", <b>last_seen</b>: "2026-03-09" }' },
          { delay: 220, type: 'json-row', html: '{ <b>email</b>: "s.okonkwo@helix.io", <b>plan</b>: "pro", <b>last_seen</b>: "2026-03-12" }' },
          { delay: 220, type: 'json-row', html: '{ <b>email</b>: "m.tanaka@yamashita.jp", <b>plan</b>: "pro", <b>last_seen</b>: "2026-03-18" }' },
          { delay: 500, type: 'ai', html: '<b>219</b> Pro users haven\'t logged in for 30+ days.' },
        ],
      },

      ssh: {
        label: 'SSH', pill: 'SSH',
        sideCta: 'New SSH Profile', sideCards: [],
        sideListTitle: 'PROFILES',
        sideListItems: [
          { glyph: 'B', name: 'box.atlas.dev', meta: 'pi@ · ed25519 · keychain', active: true, tag: 'SFTP' },
          { glyph: 'E', name: 'edge-eu-1',     meta: 'deploy@ · agent fwd', tag: 'SFTP' },
        ],
        aiPlaceholder: 'e.g. show disk usage on this server',
        sbLeft: 'pi@box.atlas.dev',
        sbCenter: '<span class="sb-chip">pty 132×38</span><span class="sb-chip">ed25519</span>',
        workspace: `
          <div class="ws-tabbar"><div class="ws-tab is-active"><i class="fa-solid fa-terminal"></i> pi@box.atlas.dev<i class="fa-solid fa-xmark"></i></div></div>
          <div class="ws-scroll ssh-pane">
            <pre class="term" data-term><span class="t-muted">Linux box.atlas.dev 6.1.0-arm64 #1 SMP PREEMPT</span>
<span class="prompt">pi@box ~ $</span> </pre>
          </div>
        `,
        script: [
          { delay: 400, type: 'user-type', text: 'What\'s eating RAM on box.atlas.dev?' },
          { delay: 700, type: 'ai', html: 'Proposing a <b>read-only</b> command — you approve before it runs.' },
          { delay: 600, type: 'term-line', html: '<span class="prompt">pi@box ~ $</span> ps -eo pid,rss,comm --sort=-rss | head -5' },
          { delay: 220, type: 'term-line', html: '<span class="t-muted">  PID   RSS COMMAND</span>' },
          { delay: 220, type: 'term-line', html: ' 1842 <span class="warn">2483120</span> node' },
          { delay: 220, type: 'term-line', html: ' 1190  626432 redis-server' },
          { delay: 220, type: 'term-line', html: ' 1027  187904 nginx' },
          { delay: 500, type: 'ai', html: 'Top: <b>node</b> 2.4 GB · <b>redis</b> 612 MB · <b>nginx</b> 184 MB.' },
        ],
      },

      explorer: {
        label: 'Explorer', pill: 'EXPLORER',
        sideCta: 'New Connection', sideCards: [],
        sideListTitle: 'CONNECTIONS',
        sideListItems: [
          { glyph: 'B', name: 'box.atlas.dev', meta: 'SFTP · /var/log', active: true, tag: 'SFTP' },
          { glyph: 'A', name: 'atlas-backups', meta: 'S3 · eu-central-1', tag: 'S3' },
        ],
        aiPlaceholder: 'e.g. show large files modified today',
        sbLeft: 'box.atlas.dev : /var/log',
        sbCenter: '<span class="sb-chip">18 entries</span>',
        workspace: `
          <div class="ws-tabbar"><div class="ws-tab is-active"><i class="fa-solid fa-folder"></i> /var/log<i class="fa-solid fa-xmark"></i></div></div>
          <div class="ws-scroll exp-pane">
            <table class="exp-table">
              <thead><tr><th>Name</th><th>Size</th><th>Modified</th><th>Perms</th></tr></thead>
              <tbody data-exp-rows></tbody>
            </table>
          </div>
        `,
        script: [
          { delay: 400, type: 'user-type', text: 'What\'s growing fast in /var/log?' },
          { delay: 700, type: 'ai', html: 'Listing <code>/var/log</code>, sorted by modified...' },
          { delay: 500, type: 'exp-row', html: '<td><i class="fa-solid fa-folder"></i> nginx</td><td>4.0 KB</td><td>21:48</td><td class="mono">drwxr-xr-x</td>' },
          { delay: 220, type: 'exp-row', html: '<td><i class="fa-solid fa-file-lines"></i> syslog</td><td class="warn">12.4 MB</td><td>21:51</td><td class="mono">-rw-r-----</td>' },
          { delay: 220, type: 'exp-row', html: '<td><i class="fa-solid fa-file-lines"></i> auth.log</td><td>814 KB</td><td>21:18</td><td class="mono">-rw-r-----</td>' },
          { delay: 220, type: 'exp-row', html: '<td><i class="fa-solid fa-file-lines"></i> dpkg.log</td><td>91 KB</td><td>03:01</td><td class="mono">-rw-r--r--</td>' },
          { delay: 500, type: 'ai', html: '<b>syslog</b> grew to <b>12.4 MB</b> in the last 2 hours.' },
        ],
      },

      history: {
        label: 'History', pill: 'HISTORY',
        sideCta: 'Clear', sideCards: [],
        sideListTitle: 'FILTER',
        sideListItems: [
          { glyph: '*', name: 'All activity',   meta: '38 events', active: true },
          { glyph: 'A', name: 'Agent',          meta: '12 events' },
          { glyph: 'R', name: 'REST',           meta: '8 events' },
          { glyph: 'S', name: 'SQL · NoSQL',    meta: '14 events' },
        ],
        aiPlaceholder: 'e.g. what did I do yesterday?',
        sbLeft: 'history · last 24h',
        sbCenter: '<span class="sb-chip">38 events</span>',
        workspace: `
          <div class="ws-tabbar"><div class="ws-tab is-active"><i class="fa-solid fa-clock-rotate-left"></i> Last 24 hours<i class="fa-solid fa-xmark"></i></div></div>
          <div class="ws-scroll hist-pane">
            <div class="hist-day">Today</div>
            <div class="hist-row"><span class="hist-time">2m</span><span class="hist-pill">AGENT</span><span>Patched <code>confirmIntent</code> in payments-refactor</span></div>
            <div class="hist-row"><span class="hist-time">14m</span><span class="hist-pill">REST</span><span>Ran Atlas Smoke Tests · 5/6 passed</span></div>
            <div class="hist-row"><span class="hist-time">37m</span><span class="hist-pill">SQL</span><span>Top 5 customers by revenue · Q2</span></div>
            <div class="hist-row"><span class="hist-time">1h</span><span class="hist-pill">NOSQL</span><span>Pro users inactive 30d · 219 docs</span></div>
            <div class="hist-row"><span class="hist-time">2h</span><span class="hist-pill">SSH</span><span>Inspected RAM on box.atlas.dev</span></div>
          </div>
        `,
        script: [
          { delay: 600, type: 'ai', html: 'Showing the last 24 hours across all modes — every prompt, query, request, and shell command stays here, scoped to your laptop.' },
        ],
      },
    };

    /* render rail (8 buttons) — real Clauge sidebar SVGs */
    if (railHost) {
      railHost.innerHTML = ORDER.map(m => {
        const def = MODES[m];
        return `<button class="rail-btn" data-mode="${m}" role="tab" aria-selected="false" type="button">
          <span class="rail-svg">${RAIL_SVG[m] || ''}</span>
          <span class="rail-label">${def.label}</span>
        </button>`;
      }).join('');
    }

    const setRail = (mode) => {
      $$('.rail-btn', railHost).forEach(b => {
        const on = b.dataset.mode === mode;
        b.classList.toggle('is-active', on);
        b.setAttribute('aria-selected', on ? 'true' : 'false');
      });
    };

    const renderTabs = (def) => {
      // tabbar mirrors first ws-tab from workspace HTML; we keep it empty since the inner ws-tabbar covers tabs.
      if (tabbar) tabbar.innerHTML = `<div class="tab is-active"><span>${def.label.toLowerCase()}</span></div>`;
    };

    const renderSidebar = (def) => {
      if (sideCtaLbl) sideCtaLbl.textContent = def.sideCta || 'New';
      if (sideCards) {
        sideCards.innerHTML = (def.sideCards || []).map(c =>
          `<div class="side-card"><i class="fa-solid ${c.icon}"></i><span>${c.label}</span>${c.badge ? `<span class="side-badge">${c.badge}</span>` : ''}</div>`
        ).join('');
        sideCards.style.display = (def.sideCards && def.sideCards.length) ? 'grid' : 'none';
      }
      if (sideList) {
        const itemsHtml = (def.sideListItems || []).map(it =>
          `<div class="side-item ${it.active ? 'is-active' : ''}">
            <span class="side-glyph">${it.glyph}</span>
            <div class="side-text"><div class="side-name">${it.name}</div><div class="side-meta">${it.meta || ''}</div></div>
            ${it.badge ? `<span class="side-progress">${it.badge}</span>` : ''}
            ${it.tag ? `<span class="side-tag">${it.tag}</span>` : ''}
          </div>`
        ).join('');
        sideList.innerHTML = `<div class="side-list-title">${def.sideListTitle || ''}</div>${itemsHtml}`;
      }
    };

    let timers = [];
    const clearScript = () => {
      timers.forEach(t => clearTimeout(t));
      timers = [];
      if (aiStream) aiStream.innerHTML = '';
    };

    const activeStream = () => workspace.querySelector('[data-agent-stream]') || aiStream;

    const playStep = (step) => {
      if (step.type === 'user' || step.type === 'ai' || step.type === 'agent' || step.type === 'thinking') {
        const stream = activeStream();
        if (!stream) return;
        const b = document.createElement('div');
        b.className = 'bubble b-' + step.type;
        b.innerHTML = step.html;
        stream.appendChild(b);
        while (stream.children.length > 7) stream.removeChild(stream.firstChild);
        stream.scrollTop = stream.scrollHeight;
        return;
      }
      if (step.type === 'user-type') {
        const targets = [];
        const inPaneInput = workspace.querySelector('[data-ag-input]');
        if (inPaneInput) targets.push(inPaneInput);
        else if (aiInput) targets.push(aiInput);
        if (step.target) {
          const el = workspace.querySelector(step.target);
          if (el) targets.push(el);
        }
        typeInto(targets, step.text, () => {
          const stream = activeStream();
          if (!stream) return;
          const b = document.createElement('div');
          b.className = 'bubble b-user';
          b.textContent = step.text;
          stream.appendChild(b);
          while (stream.children.length > 7) stream.removeChild(stream.firstChild);
          stream.scrollTop = stream.scrollHeight;
          if (inPaneInput) inPaneInput.value = '';
        });
        return;
      }
      if (step.type === 'rest-row') {
        const host = workspace.querySelector('[data-rest-rows]');
        if (host) { const r = document.createElement('div'); r.className = 'rest-row'; r.innerHTML = step.html; host.appendChild(r); }
        return;
      }
      if (step.type === 'sql-row') {
        const host = workspace.querySelector('[data-sql-rows]');
        if (host) { const tr = document.createElement('tr'); tr.innerHTML = step.html; host.appendChild(tr); }
        return;
      }
      if (step.type === 'json-row') {
        const host = workspace.querySelector('[data-nosql-rows]');
        if (host) { const d = document.createElement('div'); d.className = 'json-doc'; d.innerHTML = step.html; host.appendChild(d); }
        return;
      }
      if (step.type === 'term-line') {
        const host = workspace.querySelector('[data-term]');
        if (host) { host.insertAdjacentHTML('beforeend', '\n' + step.html); host.scrollTop = host.scrollHeight; }
        return;
      }
      if (step.type === 'exp-row') {
        const host = workspace.querySelector('[data-exp-rows]');
        if (host) { const tr = document.createElement('tr'); tr.innerHTML = step.html; host.appendChild(tr); }
        return;
      }
      if (step.type === 'bd-pop') {
        const card = workspace.querySelector('[data-bd-assigning]');
        if (card) card.classList.add('is-picking');
        return;
      }
      if (step.type === 'bd-assign') {
        const slot = workspace.querySelector('[data-bd-cw]');
        if (slot) { slot.classList.remove('bd-cw-empty'); slot.innerHTML = `<span class="bd-cw-avatar">${step.text}</span>${step.name}`; }
        const card = workspace.querySelector('[data-bd-assigning]');
        if (card) card.classList.remove('is-picking');
        return;
      }
    };

    const typeInto = (targets, text, done) => {
      let i = 0;
      const tick = () => {
        i++;
        targets.forEach(t => {
          if (!t) return;
          if ('value' in t && t.tagName === 'INPUT') t.value = text.slice(0, i);
          else t.textContent = text.slice(0, i);
        });
        if (i < text.length) timers.push(setTimeout(tick, 28 + Math.random() * 20));
        else if (done) timers.push(setTimeout(done, 150));
      };
      tick();
    };

    const playScript = (mode) => {
      clearScript();
      const def = MODES[mode];
      const steps = def.script || [];
      let cum = 0;
      steps.forEach(step => { cum += step.delay || 0; timers.push(setTimeout(() => playStep(step), cum)); });
    };

    const setMode = (mode) => {
      const def = MODES[mode];
      if (!def) return;
      stage.dataset.mode = mode;
      setRail(mode);
      renderTabs(def);
      renderSidebar(def);
      if (workspace) workspace.innerHTML = def.workspace || '';
      if (aiPill) aiPill.textContent = def.pill;
      if (aiInput) { aiInput.value = ''; aiInput.placeholder = def.aiPlaceholder || 'ask anything'; }
      if (sbLeft)   sbLeft.textContent   = def.sbLeft   || '';
      if (sbCenter) sbCenter.innerHTML   = def.sbCenter || '';
      playScript(mode);
    };

    railHost.addEventListener('click', (e) => {
      const btn = e.target.closest('.rail-btn');
      if (!btn) return;
      manualOverride();
      setMode(btn.dataset.mode);
    });

    let cycleIndex = 0;
    let cycleTimer = null;
    let pausedUntil = 0;
    const CYCLE_MS = 11000;

    const tick = () => {
      if (Date.now() < pausedUntil) return;
      cycleIndex = (cycleIndex + 1) % ORDER.length;
      setMode(ORDER[cycleIndex]);
    };
    const startCycle = () => { if (reduceMotion || cycleTimer) return; cycleTimer = setInterval(tick, CYCLE_MS); };
    const stopCycle  = () => { if (cycleTimer) { clearInterval(cycleTimer); cycleTimer = null; } };
    const manualOverride = () => {
      pausedUntil = Date.now() + 16000;
      cycleIndex = ORDER.indexOf(stage.dataset.mode || 'agent');
    };

    stage.addEventListener('mouseenter', stopCycle);
    stage.addEventListener('mouseleave', startCycle);
    document.addEventListener('visibilitychange', () => { if (document.hidden) stopCycle(); else startCycle(); });

    if ('IntersectionObserver' in window) {
      const io = new IntersectionObserver((entries) => {
        for (const e of entries) { if (e.isIntersecting) startCycle(); else stopCycle(); }
      }, { threshold: 0.25 });
      io.observe(stage);
    } else {
      startCycle();
    }

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
