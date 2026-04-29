/**
 * Parses a URL string and returns HTML with syntax coloring.
 * Colors: protocol, domain, path, query keys/values, variables as chips.
 */

function escapeHtml(str: string): string {
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export function highlightUrl(raw: string): string {
  if (!raw) return '';

  // First, split by variable patterns {{...}}
  const parts = raw.split(/(\{\{[^}]*\}\})/g);
  let assembled = '';

  for (const part of parts) {
    if (part.startsWith('{{') && part.endsWith('}}')) {
      assembled += `<span class="url-var">${escapeHtml(part)}</span>`;
    } else {
      assembled += colorUrlPart(part);
    }
  }

  return assembled;
}

function colorUrlPart(text: string): string {
  if (!text) return '';

  let result = '';
  let remaining = text;

  // Protocol
  const protoMatch = remaining.match(/^(https?:\/\/)/);
  if (protoMatch) {
    result += `<span class="url-proto">${escapeHtml(protoMatch[1])}</span>`;
    remaining = remaining.slice(protoMatch[1].length);
  }

  // Split at query string
  const qIdx = remaining.indexOf('?');
  const beforeQuery = qIdx >= 0 ? remaining.slice(0, qIdx) : remaining;
  const queryPart = qIdx >= 0 ? remaining.slice(qIdx) : '';

  // Host + path
  const slashIdx = beforeQuery.indexOf('/');
  if (slashIdx >= 0) {
    const host = beforeQuery.slice(0, slashIdx);
    const path = beforeQuery.slice(slashIdx);
    result += `<span class="url-host">${escapeHtml(host)}</span>`;
    result += `<span class="url-path">${escapeHtml(path)}</span>`;
  } else {
    result += `<span class="url-host">${escapeHtml(beforeQuery)}</span>`;
  }

  // Query string
  if (queryPart) {
    result += `<span class="url-qmark">?</span>`;
    const pairs = queryPart.slice(1).split('&');
    for (let i = 0; i < pairs.length; i++) {
      if (i > 0) result += `<span class="url-qmark">&amp;</span>`;
      const eqIdx = pairs[i].indexOf('=');
      if (eqIdx >= 0) {
        result += `<span class="url-qkey">${escapeHtml(pairs[i].slice(0, eqIdx))}</span>`;
        result += `<span class="url-qmark">=</span>`;
        result += `<span class="url-qval">${escapeHtml(pairs[i].slice(eqIdx + 1))}</span>`;
      } else {
        result += `<span class="url-qkey">${escapeHtml(pairs[i])}</span>`;
      }
    }
  }

  return result;
}

/**
 * Highlights {{variables}} in a plain text string (for header values etc).
 */
export function highlightVars(raw: string): string {
  if (!raw) return '';
  return raw.replace(/(\{\{[^}]*\}\})/g, (_, m) => {
    return `<span class="url-var">${escapeHtml(m)}</span>`;
  });
}
