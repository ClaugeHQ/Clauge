// SSH-mode safety helpers.
//
// Per design (see docs/superpowers/specs/2026-04-27-ssh-mode-design.md and the
// SSH_SYSTEM_PROMPT) we deliberately do NOT keep a hardcoded denylist of
// destructive commands. Static lists are always incomplete and create false
// confidence. Safety in SSH mode is layered:
//   1. Strong system prompt — AI refuses destructive ops
//   2. User confirmation modal on every execute_shell call
//   3. Output redaction (this module) before stdout returns to the model
//
// The only programmatic safety here is `redactSecrets`, applied to captured
// terminal output before it is sent back as a tool result.

const REDACTION = '[REDACTED]';

// Patterns ordered from specific → general. Each replaces the secret-looking
// substring with REDACTION while preserving surrounding text so the AI can
// still understand structure.
const REDACTION_PATTERNS: Array<{ re: RegExp; replace: (match: string, ...g: string[]) => string }> = [
  // JWT (3 base64url segments separated by dots)
  { re: /eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, replace: () => REDACTION },

  // key=value where key looks secret-like (token, password, secret, api[_-]?key, etc.)
  // Captures the assignment operator so we keep the structure visible.
  {
    re: /\b((?:[A-Za-z][\w-]*)?(?:password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|auth)[\w-]*)\s*([=:])\s*("[^"\n]*"|'[^'\n]*'|\S+)/gi,
    replace: (_m, key, op) => `${key}${op}${REDACTION}`,
  },

  // Bearer / Basic auth headers
  { re: /\b(Authorization|Bearer|Basic)\s+([A-Za-z0-9._\-+/=]{16,})/g, replace: (_m, kind) => `${kind} ${REDACTION}` },

  // Long hex strings (likely API keys / tokens, 32+ hex chars)
  { re: /\b[a-fA-F0-9]{32,}\b/g, replace: () => REDACTION },

  // AWS-style access keys
  { re: /\b(AKIA|ASIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA|ASCA)[A-Z0-9]{16}\b/g, replace: () => REDACTION },

  // GitHub-style PATs and similar
  { re: /\b(ghp|gho|ghu|ghs|ghr|github_pat)_[A-Za-z0-9_]{30,}/g, replace: () => REDACTION },
];

/**
 * Redact strings that look like secrets from text captured from the SSH
 * terminal before forwarding it to the AI provider. Best-effort — not a
 * substitute for the user being mindful about what's on screen, but
 * catches the common cases (env vars, tokens, JWTs, AWS keys, GH PATs).
 */
export function redactSecrets(text: string): string {
  if (!text) return text;
  let out = text;
  for (const { re, replace } of REDACTION_PATTERNS) {
    out = out.replace(re, replace);
  }
  return out;
}
