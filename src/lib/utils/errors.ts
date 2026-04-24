/**
 * Convert raw error messages to user-friendly text.
 * Strips Rust error prefixes, technical jargon, and stack traces.
 * Always logs the raw error to console for debugging.
 */
export function friendlyError(err: unknown): string {
  // Always log the raw error for debugging
  console.error('[Clauge Error]', err);

  let msg = '';
  if (err instanceof Error) {
    msg = err.message;
  } else if (typeof err === 'string') {
    msg = err;
  } else {
    msg = String(err);
  }

  // Strip common Rust/Tauri prefixes
  msg = msg.replace(/^error\[.*?\]:\s*/i, '');
  msg = msg.replace(/^Error:\s*/i, '');

  // HTTP request errors — extract root cause from reqwest's nested format:
  // "Request failed: error sending request for url (https://...): <actual cause>"
  if (msg.includes('error sending request for url')) {
    // Extract the cause after the last ):
    const causeMatch = msg.match(/error sending request for url \([^)]+\):\s*(.+)/i);
    if (causeMatch) {
      msg = causeMatch[1]; // Use the actual cause for further matching below
    }
  }

  // DNS resolution errors
  if (msg.includes('dns error') || msg.includes('failed to lookup') || msg.includes('getaddrinfo') || msg.includes('Name or service not known') || msg.includes('No address associated')) {
    return 'DNS resolution failed — check the hostname or your network/VPN';
  }

  // Connection errors
  if (msg.includes('Connection refused') || msg.includes('connection refused')) {
    return 'Could not connect — is the server running?';
  }
  if (msg.includes('password authentication failed')) {
    return 'Authentication failed — check your username and password';
  }
  if (msg.includes('Connection not found') || msg.includes('connection not found')) {
    return 'Connection lost — please disconnect and reconnect';
  }
  if (msg.includes('timeout') || msg.includes('timed out')) {
    return 'Connection timed out — check your network and server';
  }
  if (msg.includes('SSL') || msg.includes('ssl') || msg.includes('TLS')) {
    return 'SSL connection error — check your SSL settings';
  }
  if (msg.includes('ECONNREFUSED') || msg.includes('ENOTFOUND')) {
    return 'Server not reachable — check host and port';
  }

  // SQL errors — check specific patterns BEFORE generic "does not exist"
  if (msg.includes('syntax error')) {
    const match = msg.match(/syntax error at or near "([^"]+)"/);
    return match ? `SQL syntax error near "${match[1]}"` : 'SQL syntax error — check your query';
  }
  if (msg.includes('relation') && msg.includes('does not exist')) {
    const match = msg.match(/relation "([^"]+)" does not exist/);
    return match ? `Table "${match[1]}" not found` : 'Table not found';
  }
  if (msg.includes('column') && msg.includes('does not exist')) {
    const match = msg.match(/column "([^"]+)" does not exist/);
    return match ? `Column "${match[1]}" not found` : 'Column not found';
  }
  // Database not found — use specific regex to avoid matching "error returned from database"
  if (/database "([^"]+)" does not exist/.test(msg)) {
    const match = msg.match(/database "([^"]+)" does not exist/);
    return match ? `Database "${match[1]}" not found` : 'Database not found';
  }
  if (msg.includes('permission denied')) {
    return 'Permission denied — insufficient privileges';
  }
  if (msg.includes('duplicate key')) {
    return 'Duplicate entry — a record with this key already exists';
  }
  if (msg.includes('violates foreign key')) {
    return 'Cannot complete — referenced record does not exist or is still in use';
  }
  if (msg.includes('violates not-null')) {
    return 'Required field is missing — cannot be empty';
  }
  if (msg.includes('multiple commands') || msg.includes('prepared statement')) {
    return 'Cannot execute multiple statements at once — select the query you want to run';
  }
  if (msg.includes('unterminated') || msg.includes('incomplete')) {
    return 'Incomplete query — check for missing keywords or semicolons';
  }

  // MongoDB errors — detect by error patterns (Rust driver doesn't always include "mongo")
  if (msg.includes('MongoServerError') || msg.includes('mongo') || msg.includes('MongoDB')
      || msg.includes('$clusterTime') || msg.includes('$db') || msg.includes('not authorized on')) {
    msg = msg.replace(/MongoServerError:\s*/, '');
    // Auth / permission failures
    if (msg.includes('not authorized') || msg.includes('auth') || msg.includes('Authentication')
        || msg.includes('Unauthorized') || msg.includes('Error code 13')) {
      const dbMatch = msg.match(/not authorized on (\w+)/);
      if (dbMatch) {
        return `Not authorized on "${dbMatch[1]}" — insufficient permissions`;
      }
      return 'MongoDB authentication failed — check your username, password, and auth database';
    }
    if (msg.includes('ping failed') || msg.includes('Server selection timeout')) {
      if (msg.includes('unexpected end of file') || msg.includes('I/O error')) {
        return 'Could not reach MongoDB server — try enabling SSL/TLS';
      }
      return 'Could not reach MongoDB server — check host, port, and credentials';
    }
    // Find/query errors — extract the useful part
    if (msg.includes('Find error') || msg.includes('Command failed')) {
      const codeMatch = msg.match(/Error code \d+ \((\w+)\)/);
      if (codeMatch) {
        return `MongoDB error — ${codeMatch[1]}`;
      }
    }
    return 'MongoDB operation failed — check console for details';
  }

  // Redis errors
  if (msg.includes('WRONGTYPE')) {
    return 'Wrong data type for this operation';
  }
  if (msg.includes('NOAUTH') || msg.includes('Authentication required')) {
    return 'Redis authentication required — check your password';
  }
  if (msg.includes('Redis')) {
    if (msg.includes('auth') || msg.includes('401') || msg.includes('Unauthorized')) {
      return 'Redis authentication failed — check your password';
    }
  }

  // GitHub/Gist errors (only match when not a DB error)
  if ((msg.includes('401') || msg.includes('Unauthorized')) && (msg.includes('GitHub') || msg.includes('token') || msg.includes('gist'))) {
    return 'Invalid or expired token — please reconnect';
  }
  if (msg.includes('404') || msg.includes('Not Found')) {
    return 'Resource not found';
  }
  if (msg.includes('rate limit')) {
    return 'Rate limit exceeded — try again later';
  }

  // Generic fallback — show the actual error message, cleaned up
  // Strip "error returned from database: " prefix
  msg = msg.replace(/^error returned from database:\s*/i, '');
  // Capitalize first letter
  if (msg.length > 0) {
    msg = msg.charAt(0).toUpperCase() + msg.slice(1);
  }
  // Truncate if too long
  if (msg.length > 120) {
    msg = msg.substring(0, 117) + '...';
  }
  return msg || 'Something went wrong';
}
