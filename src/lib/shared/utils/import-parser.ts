/**
 * Parse CSV text into an array of objects.
 * Handles: quoted fields, commas inside quotes, newlines inside quotes,
 * escaped quotes (""), leading/trailing whitespace, empty fields, BOM,
 * mixed line endings (\r\n, \n, \r).
 */
export function parseCsv(text: string): Record<string, string>[] {
  // Strip BOM
  const clean = text.startsWith('\ufeff') ? text.slice(1) : text;

  const rows = parseCsvRows(clean);
  if (rows.length < 2) return [];

  const headers = rows[0].map(h => h.trim());
  const docs: Record<string, string>[] = [];

  for (let i = 1; i < rows.length; i++) {
    const values = rows[i];
    // Skip rows where all fields are empty
    if (values.every(v => v.trim() === '')) continue;
    const doc: Record<string, string> = {};
    headers.forEach((h, j) => {
      if (h) doc[h] = (values[j] ?? '').trim();
    });
    docs.push(doc);
  }

  return docs;
}

/**
 * Parse CSV text into rows of fields, handling RFC 4180 quoting rules.
 */
function parseCsvRows(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = '';
  let inQuotes = false;
  let i = 0;
  const len = text.length;

  while (i < len) {
    const ch = text[i];

    if (inQuotes) {
      if (ch === '"') {
        // Check for escaped quote ""
        if (i + 1 < len && text[i + 1] === '"') {
          field += '"';
          i += 2;
        } else {
          // End of quoted field
          inQuotes = false;
          i++;
        }
      } else {
        field += ch;
        i++;
      }
    } else {
      if (ch === '"') {
        inQuotes = true;
        i++;
      } else if (ch === ',') {
        row.push(field);
        field = '';
        i++;
      } else if (ch === '\r') {
        // Handle \r\n or standalone \r
        row.push(field);
        field = '';
        rows.push(row);
        row = [];
        i++;
        if (i < len && text[i] === '\n') i++;
      } else if (ch === '\n') {
        row.push(field);
        field = '';
        rows.push(row);
        row = [];
        i++;
      } else {
        field += ch;
        i++;
      }
    }
  }

  // Last field/row
  if (field || row.length > 0) {
    row.push(field);
    rows.push(row);
  }

  return rows;
}

/**
 * Parse JSON text into an array of documents.
 * Handles: single object, array of objects, JSONL (one JSON per line),
 * trailing commas (best effort), whitespace/empty lines.
 */
export function parseJsonDocs(text: string): any[] {
  const trimmed = text.trim();
  if (!trimmed) return [];

  // Try standard JSON parse first (array or single object)
  try {
    const parsed = JSON.parse(trimmed);
    if (Array.isArray(parsed)) return parsed;
    if (typeof parsed === 'object' && parsed !== null) return [parsed];
    return [];
  } catch {
    // Not valid JSON — try JSONL (one JSON object per line)
  }

  // Try JSONL: split by lines, parse each independently
  const lines = trimmed.split('\n').map(l => l.trim()).filter(l => l && !l.startsWith('//'));
  const docs: any[] = [];
  for (const line of lines) {
    try {
      const parsed = JSON.parse(line);
      if (typeof parsed === 'object' && parsed !== null) {
        docs.push(parsed);
      }
    } catch {
      // Skip unparseable lines
    }
  }

  if (docs.length > 0) return docs;

  // Try removing trailing commas and re-parse as array
  try {
    const fixed = '[' + trimmed.replace(/,\s*([}\]])/g, '$1').replace(/^\[|\]$/g, '') + ']';
    const parsed = JSON.parse(fixed);
    if (Array.isArray(parsed)) return parsed;
  } catch {
    // Give up
  }

  throw new Error('Could not parse file as JSON. Expected a JSON object, array, or JSONL format.');
}
