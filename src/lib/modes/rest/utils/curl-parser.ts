import type { KVInput } from '$lib/types';

export interface ParsedCurl {
  method: string;
  url: string;
  headers: KVInput[];
  body: string;
  bodyType: string;
  authType: string;
  authData: string;
}

// Flags that take no value (boolean flags)
const BOOLEAN_FLAGS = new Set([
  '-#', '-0', '-1', '-2', '-3', '-4', '-6', '-a', '-B', '-C',
  '-f', '-g', '-G', '-i', '-I', '-j', '-J', '-k', '-l', '-L',
  '-n', '-N', '-O', '-p', '-P', '-q', '-R', '-s', '-S', '-v', '-V',
  '--anyauth', '--basic', '--compressed', '--create-dirs', '--crlf',
  '--digest', '--disable-eprt', '--disable-epsv', '--fail',
  '--fail-early', '--false-start', '--ftp-create-dirs',
  '--ftp-pasv', '--ftp-skip-pasv-ip', '--ftp-ssl-ccc',
  '--ftp-ssl-control', '--globoff', '--head', '--help',
  '--http1.0', '--http1.1', '--http2', '--http2-prior-knowledge',
  '--ignore-content-length', '--include', '--insecure',
  '--ipv4', '--ipv6', '--junk-session-cookies', '--list-only',
  '--location', '--location-trusted', '--manual',
  '--metalink', '--negotiate', '--netrc', '--netrc-optional',
  '--no-alpn', '--no-buffer', '--no-keepalive', '--no-npn',
  '--no-sessionid', '--ntlm', '--ntlm-wb', '--path-as-is',
  '--post301', '--post302', '--post303', '--progress-bar',
  '--proto-default', '--proxy-anyauth', '--proxy-basic',
  '--proxy-digest', '--proxy-negotiate', '--proxy-ntlm',
  '--raw', '--remote-header-name', '--remote-name',
  '--remote-name-all', '--remote-time', '--retry-connrefused',
  '--sasl-ir', '--show-error', '--silent', '--socks5-gssapi-nec',
  '--ssl', '--ssl-allow-beast', '--ssl-no-revoke', '--ssl-reqd',
  '--stderr', '--styled-output', '--suppress-connect-headers',
  '--tcp-fastopen', '--tcp-nodelay', '--tftp-no-options',
  '--tlsv1', '--tlsv1.0', '--tlsv1.1', '--tlsv1.2', '--tlsv1.3',
  '--tr-encoding', '--trace-time', '--use-ascii', '--verbose',
  '--version', '--xattr',
]);

// Flags that take a value argument
const VALUE_FLAGS = new Set([
  '-A', '-e', '-o', '-x', '-E', '-K', '-w', '-T', '-Q',
  '--abstract-unix-socket', '--cacert', '--capath', '--cert',
  '--cert-type', '--ciphers', '--connect-timeout', '--connect-to',
  '--continue-at', '--cookie-jar', '--crlfile', '--data-ascii',
  '--delegation', '--dns-interface', '--dns-ipv4-addr',
  '--dns-ipv6-addr', '--dns-servers', '--engine', '--expect100-timeout',
  '--form-string', '--ftp-account', '--ftp-alternative-to-user',
  '--ftp-method', '--ftp-port', '--ftp-ssl-ccc-mode',
  '--happy-eyeballs-timeout-ms', '--hostpubmd5', '--interface',
  '--keepalive-time', '--key', '--key-type', '--krb',
  '--limit-rate', '--local-port', '--login-options',
  '--mail-auth', '--mail-from', '--mail-rcpt', '--max-filesize',
  '--max-redirs', '--max-time', '--noproxy', '--output',
  '--pass', '--pinnedpubkey', '--proto', '--proto-redir',
  '--proxy', '--proxy-cacert', '--proxy-capath', '--proxy-cert',
  '--proxy-cert-type', '--proxy-ciphers', '--proxy-crlfile',
  '--proxy-header', '--proxy-key', '--proxy-key-type',
  '--proxy-pass', '--proxy-service-name', '--proxy-tls13-ciphers',
  '--proxy-tlsauthtype', '--proxy-tlspassword', '--proxy-tlsuser',
  '--proxy-user', '--pubkey', '--random-file', '--range',
  '--referer', '--resolve', '--retry', '--retry-delay',
  '--retry-max-time', '--service-name', '--socks4',
  '--socks4a', '--socks5', '--socks5-gssapi-service',
  '--socks5-hostname', '--speed-limit', '--speed-time',
  '--telnet-option', '--tftp-blksize', '--time-cond',
  '--tls-max', '--tls13-ciphers', '--tlsauthtype',
  '--tlspassword', '--tlsuser', '--trace', '--trace-ascii',
  '--unix-socket', '--upload-file', '--url', '--user-agent',
  '--write-out',
]);

/**
 * Split a --flag=value token into [flag, value].
 * Returns null if not in --flag=value format.
 */
function splitEqualsFlag(tok: string): [string, string] | null {
  if (!tok.startsWith('-')) return null;
  const eqIdx = tok.indexOf('=');
  if (eqIdx <= 0) return null;
  return [tok.substring(0, eqIdx), tok.substring(eqIdx + 1)];
}

/**
 * Parse a cURL command string into its components.
 */
export function parseCurl(text: string): ParsedCurl | null {
  // Normalize: remove backslash-newline and ^-newline (Windows) continuations
  const normalized = text
    .replace(/\\\s*\n/g, ' ')
    .replace(/\^\s*\n/g, ' ')
    .replace(/\^(\s)/g, '$1') // Remove standalone ^ used as Windows continuation
    .trim();

  if (!normalized.startsWith('curl ') && !normalized.startsWith('curl\t')) {
    return null;
  }

  const tokens = tokenize(normalized);
  tokens.shift(); // Remove 'curl'

  let method = '';
  let url = '';
  const headers: KVInput[] = [];
  const bodyParts: string[] = [];
  const formFields: { value: string; isFormString: boolean }[] = [];
  let authType = 'none';
  let authData = '{}';

  let i = 0;
  while (i < tokens.length) {
    let tok = tokens[i];

    // Handle --flag=value syntax: split into flag + value and re-process
    const eqSplit = splitEqualsFlag(tok);
    if (eqSplit) {
      tok = eqSplit[0];
      // Insert the value back into tokens so the flag handler can consume it
      tokens.splice(i + 1, 0, eqSplit[1]);
    }

    // Handle -XMETHOD (combined -X with method, no space)
    if (tok.startsWith('-X') && tok.length > 2 && tok !== '-X') {
      method = tok.substring(2).toUpperCase();
      i++;
      continue;
    }

    if (tok === '-X' || tok === '--request') {
      i++;
      if (i < tokens.length) method = tokens[i].toUpperCase();
    } else if (tok === '-H' || tok === '--header') {
      i++;
      if (i < tokens.length) {
        const hdr = tokens[i];
        const colonIdx = hdr.indexOf(':');
        if (colonIdx > 0) {
          const key = hdr.substring(0, colonIdx).trim();
          const value = hdr.substring(colonIdx + 1).trim();
          headers.push({ key, value, enabled: 1 });
        }
      }
    } else if (tok === '--json') {
      // cURL 7.82+ --json flag: sets Content-Type + Accept to JSON, sends body
      i++;
      if (i < tokens.length) bodyParts.push(tokens[i]);
      // Add JSON headers if not already present
      if (!headers.some(h => h.key.toLowerCase() === 'content-type')) {
        headers.push({ key: 'Content-Type', value: 'application/json', enabled: 1 });
      }
      if (!headers.some(h => h.key.toLowerCase() === 'accept')) {
        headers.push({ key: 'Accept', value: 'application/json', enabled: 1 });
      }
    } else if (tok === '-d' || tok === '--data' || tok === '--data-raw' || tok === '--data-binary' || tok === '--data-ascii') {
      i++;
      if (i < tokens.length) bodyParts.push(tokens[i]);
    } else if (tok === '--data-urlencode') {
      i++;
      if (i < tokens.length) {
        // Properly URL-encode the value
        const raw = tokens[i];
        const eqIdx = raw.indexOf('=');
        if (eqIdx >= 0) {
          const key = raw.substring(0, eqIdx);
          const val = raw.substring(eqIdx + 1);
          bodyParts.push(`${encodeURIComponent(key)}=${encodeURIComponent(val)}`);
        } else {
          bodyParts.push(encodeURIComponent(raw));
        }
      }
    } else if (tok === '-F' || tok === '--form') {
      i++;
      if (i < tokens.length) formFields.push({ value: tokens[i], isFormString: false });
    } else if (tok === '--form-string') {
      i++;
      if (i < tokens.length) formFields.push({ value: tokens[i], isFormString: true });
    } else if (tok === '-b' || tok === '--cookie') {
      i++;
      if (i < tokens.length) {
        headers.push({ key: 'Cookie', value: tokens[i], enabled: 1 });
      }
    } else if (tok === '-u' || tok === '--user') {
      i++;
      if (i < tokens.length) {
        const cred = tokens[i];
        const sepIdx = cred.indexOf(':');
        if (sepIdx >= 0) {
          authType = 'basic';
          authData = JSON.stringify({
            username: cred.substring(0, sepIdx),
            password: cred.substring(sepIdx + 1)
          });
        }
      }
    } else if (tok === '--url') {
      i++;
      if (i < tokens.length) url = tokens[i];
    } else if (tok === '-A' || tok === '--user-agent') {
      i++;
      if (i < tokens.length) {
        headers.push({ key: 'User-Agent', value: tokens[i], enabled: 1 });
      }
    } else if (tok === '-e' || tok === '--referer') {
      i++;
      if (i < tokens.length) {
        headers.push({ key: 'Referer', value: tokens[i], enabled: 1 });
      }
    } else if (tok.startsWith('-')) {
      // Check if it's a known boolean flag (no value)
      if (BOOLEAN_FLAGS.has(tok)) {
        // Skip, no value to consume
      } else if (VALUE_FLAGS.has(tok)) {
        // Known flag with value — skip the value
        i++;
      } else if (tok.startsWith('--')) {
        // Unknown long flag — peek if next token looks like a value
        if (i + 1 < tokens.length && !tokens[i + 1].startsWith('-')) {
          i++;
        }
      } else {
        // Unknown short flag(s) like -sS, -vvv — could be combined boolean flags
        // Don't consume next token (it's likely the URL or another flag)
      }
    } else {
      // Positional argument — treat as URL if we don't have one yet
      if (!url) {
        url = tok;
      }
    }

    i++;
  }

  // Infer method from body if not explicitly set
  if (!method) {
    method = (bodyParts.length > 0 || formFields.length > 0) ? 'POST' : 'GET';
  }

  // Check for Authorization header → extract auth info
  const authHeaderIdx = headers.findIndex(
    h => h.key.toLowerCase() === 'authorization'
  );
  if (authHeaderIdx >= 0 && authType === 'none') {
    const authValue = headers[authHeaderIdx].value;
    if (authValue.toLowerCase().startsWith('bearer ')) {
      authType = 'bearer';
      authData = JSON.stringify({ token: authValue.substring(7).trim() });
      headers.splice(authHeaderIdx, 1);
    } else if (authValue.toLowerCase().startsWith('basic ')) {
      try {
        const decoded = atob(authValue.substring(6).trim());
        const sepIdx = decoded.indexOf(':');
        if (sepIdx > 0) {
          authType = 'basic';
          authData = JSON.stringify({
            username: decoded.substring(0, sepIdx),
            password: decoded.substring(sepIdx + 1)
          });
        }
      } catch {
        // Leave as header if we can't decode
      }
      if (authType === 'basic') {
        headers.splice(authHeaderIdx, 1);
      }
    }
  }

  // Detect body type from Content-Type header, flags used, or form fields
  let bodyType = 'json';
  let body = '';

  const ctHeader = headers.find(h => h.key.toLowerCase() === 'content-type');
  const ctValue = ctHeader?.value.toLowerCase() ?? '';

  if (formFields.length > 0) {
    // -F / --form → multipart, build structured JSON array
    bodyType = 'multipart';
    const multipartFields = formFields.map(f => {
      const raw = f.value;
      const eqIdx = raw.indexOf('=');
      if (eqIdx >= 0) {
        const key = raw.substring(0, eqIdx);
        const val = raw.substring(eqIdx + 1);
        // Only treat @ as file reference for --form, NOT --form-string
        if (!f.isFormString && val.startsWith('@')) {
          const filePath = val.substring(1);
          const fileName = filePath.split('/').pop() ?? 'file';
          return { key, value: '', type: 'file', filePath, fileName, enabled: true };
        }
        return { key, value: val, type: 'text', filePath: '', fileName: '', enabled: true };
      }
      return { key: raw, value: '', type: 'text', filePath: '', fileName: '', enabled: true };
    });
    body = JSON.stringify(multipartFields);

    // Override with explicit Content-Type if set
    if (ctValue.includes('application/x-www-form-urlencoded')) {
      bodyType = 'urlencoded';
      const kvFields = formFields.map(f => {
        const raw = f.value;
        const eqIdx = raw.indexOf('=');
        if (eqIdx >= 0) {
          return { key: raw.substring(0, eqIdx), value: raw.substring(eqIdx + 1), enabled: true };
        }
        return { key: raw, value: '', enabled: true };
      });
      body = JSON.stringify(kvFields);
    }
  } else if (bodyParts.length > 0) {
    const rawBody = bodyParts.join('&');

    if (ctValue) {
      // Explicit Content-Type header determines type
      if (ctValue.includes('application/json')) {
        bodyType = 'json';
        body = rawBody;
      } else if (ctValue.includes('application/x-www-form-urlencoded')) {
        bodyType = 'urlencoded';
        body = parseUrlencodedToJson(rawBody);
      } else if (ctValue.includes('text/xml') || ctValue.includes('application/xml')) {
        bodyType = 'xml';
        body = rawBody;
      } else if (ctValue.includes('text/plain')) {
        bodyType = 'text';
        body = rawBody;
      } else if (ctValue.includes('multipart/form-data')) {
        bodyType = 'multipart';
        body = rawBody;
      } else {
        bodyType = 'text';
        body = rawBody;
      }
    } else {
      // No Content-Type — try to detect from body content
      const trimmedBody = rawBody.trim();
      if ((trimmedBody.startsWith('{') && trimmedBody.endsWith('}')) ||
          (trimmedBody.startsWith('[') && trimmedBody.endsWith(']'))) {
        bodyType = 'json';
        body = rawBody;
      } else if (trimmedBody.startsWith('<')) {
        bodyType = 'xml';
        body = rawBody;
      } else if (trimmedBody.includes('=')) {
        // Looks like key=value pairs → urlencoded
        bodyType = 'urlencoded';
        body = parseUrlencodedToJson(rawBody);
      } else {
        bodyType = 'text';
        body = rawBody;
      }
    }
  }

  return { method, url, headers, body, bodyType, authType, authData };
}

/**
 * Parse a URL-encoded string (key=val&key2=val2) into a JSON array for the form editor.
 */
function parseUrlencodedToJson(raw: string): string {
  const pairs = raw.split('&').filter(Boolean);
  const fields = pairs.map(pair => {
    const eqIdx = pair.indexOf('=');
    if (eqIdx >= 0) {
      return {
        key: decodeURIComponent(pair.substring(0, eqIdx).replace(/\+/g, ' ')),
        value: decodeURIComponent(pair.substring(eqIdx + 1).replace(/\+/g, ' ')),
        enabled: true,
      };
    }
    return { key: decodeURIComponent(pair.replace(/\+/g, ' ')), value: '', enabled: true };
  });
  return JSON.stringify(fields);
}

/**
 * Tokenize a shell-like command string, respecting single and double quotes.
 */
function tokenize(input: string): string[] {
  const tokens: string[] = [];
  let i = 0;
  const len = input.length;

  while (i < len) {
    // Skip whitespace
    while (i < len && (input[i] === ' ' || input[i] === '\t')) i++;
    if (i >= len) break;

    let token = '';
    while (i < len && input[i] !== ' ' && input[i] !== '\t') {
      const ch = input[i];
      if (ch === "'") {
        // Single-quoted string: everything until next single quote
        i++;
        while (i < len && input[i] !== "'") {
          token += input[i];
          i++;
        }
        if (i < len) i++; // skip closing quote
      } else if (ch === '"') {
        // Double-quoted string: everything until next double quote, with backslash escaping
        i++;
        while (i < len && input[i] !== '"') {
          if (input[i] === '\\' && i + 1 < len) {
            const next = input[i + 1];
            if (next === '"' || next === '\\') {
              token += next;
              i += 2;
              continue;
            }
          }
          token += input[i];
          i++;
        }
        if (i < len) i++; // skip closing quote
      } else if (ch === '\\' && i + 1 < len) {
        // Escaped character
        token += input[i + 1];
        i += 2;
      } else {
        token += ch;
        i++;
      }
    }

    if (token.length > 0) {
      tokens.push(token);
    }
  }

  return tokens;
}
