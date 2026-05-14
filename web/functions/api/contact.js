// Cloudflare Pages Function — POST /api/contact
//
// Receives the enterprise contact form, validates, sends email via Resend.
//
// Required environment bindings on the Pages project:
//   RESEND_API_KEY  (secret) — from resend.com dashboard
//   CONTACT_TO      (var)    — e.g. "support@clauge.in"
//   CONTACT_FROM    (var)    — sending address. Until clauge.in is verified
//                              in Resend, use "onboarding@resend.dev"
//                              (Resend's shared sandbox sender). After
//                              verification, switch to "enterprise@clauge.in"
//                              or "support@clauge.in".

const MAX_LEN = {
  name: 120, role: 120, company: 160, email: 200, phone: 40, problem: 4000,
};

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function clean(value, max) {
  if (typeof value !== 'string') return '';
  const trimmed = value.trim().slice(0, max);
  return trimmed;
}

function escapeHtml(s) {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export async function onRequestPost(context) {
  const { request, env } = context;

  // Parse + basic validation
  let body;
  try {
    body = await request.json();
  } catch {
    return json({ error: 'Invalid JSON' }, 400);
  }

  // Honeypot: bots fill the hidden "website" field; humans don't.
  if (typeof body.website === 'string' && body.website.length > 0) {
    // Pretend success so the bot doesn't retry, but drop silently.
    return json({ ok: true }, 200);
  }

  const name    = clean(body.name,    MAX_LEN.name);
  const role    = clean(body.role,    MAX_LEN.role);
  const company = clean(body.company, MAX_LEN.company);
  const email   = clean(body.email,   MAX_LEN.email);
  const phone   = clean(body.phone,   MAX_LEN.phone);
  const problem = clean(body.problem, MAX_LEN.problem);

  if (!name || !role || !company || !email || !problem) {
    return json({ error: 'Missing required fields' }, 400);
  }
  if (!EMAIL_RE.test(email)) {
    return json({ error: 'Invalid email' }, 400);
  }

  // Build the email payload for Resend
  const ip = request.headers.get('CF-Connecting-IP') || 'unknown';
  const country = request.headers.get('CF-IPCountry') || 'unknown';
  const ua = request.headers.get('User-Agent') || 'unknown';

  const subject = `[Clauge Enterprise] ${company} — ${name}`;
  const textBody = [
    `New enterprise contact form submission:`,
    ``,
    `Name:    ${name}`,
    `Role:    ${role}`,
    `Company: ${company}`,
    `Email:   ${email}`,
    `Phone:   ${phone || '(not provided)'}`,
    ``,
    `Problem / use case:`,
    problem,
    ``,
    `--`,
    `IP: ${ip}  Country: ${country}`,
    `UA: ${ua}`,
  ].join('\n');

  const htmlBody = `
    <div style="font-family: system-ui, -apple-system, sans-serif; max-width: 640px;">
      <h2 style="margin:0 0 16px;">New enterprise contact form submission</h2>
      <table style="border-collapse: collapse; margin-bottom: 16px;">
        <tr><td style="padding: 6px 14px 6px 0; color: #888;">Name</td><td><strong>${escapeHtml(name)}</strong></td></tr>
        <tr><td style="padding: 6px 14px 6px 0; color: #888;">Role</td><td>${escapeHtml(role)}</td></tr>
        <tr><td style="padding: 6px 14px 6px 0; color: #888;">Company</td><td>${escapeHtml(company)}</td></tr>
        <tr><td style="padding: 6px 14px 6px 0; color: #888;">Email</td><td><a href="mailto:${escapeHtml(email)}">${escapeHtml(email)}</a></td></tr>
        <tr><td style="padding: 6px 14px 6px 0; color: #888;">Phone</td><td>${escapeHtml(phone) || '<em>(not provided)</em>'}</td></tr>
      </table>
      <h3 style="margin: 24px 0 8px;">Problem / use case</h3>
      <div style="white-space: pre-wrap; padding: 14px; background: #f6f6f8; border-radius: 8px; color: #222;">${escapeHtml(problem)}</div>
      <hr style="margin: 24px 0; border: none; border-top: 1px solid #e2e2e8;" />
      <p style="font-size: 12px; color: #888;">
        IP: ${escapeHtml(ip)} · Country: ${escapeHtml(country)}<br/>
        UA: ${escapeHtml(ua)}
      </p>
    </div>
  `;

  if (!env.RESEND_API_KEY) {
    return json({ error: 'Email delivery not configured' }, 500);
  }

  const to = env.CONTACT_TO || 'support@clauge.in';
  const from = env.CONTACT_FROM || 'onboarding@resend.dev';

  const resendResp = await fetch('https://api.resend.com/emails', {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${env.RESEND_API_KEY}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      from: `Clauge Enterprise <${from}>`,
      to: [to],
      reply_to: email,
      subject,
      text: textBody,
      html: htmlBody,
    }),
  });

  if (!resendResp.ok) {
    const errText = await resendResp.text().catch(() => '');
    console.error('Resend error', resendResp.status, errText);
    return json({ error: 'Could not send. Try emailing support@clauge.in directly.' }, 502);
  }

  return json({ ok: true }, 200);
}

// Reject non-POST methods cleanly
export async function onRequest(context) {
  if (context.request.method === 'POST') {
    return onRequestPost(context);
  }
  return new Response('Method not allowed', { status: 405, headers: { Allow: 'POST' } });
}

function json(body, status) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
