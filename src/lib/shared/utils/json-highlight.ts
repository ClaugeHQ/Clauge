/**
 * JSON syntax highlighting utility.
 * Takes raw JSON string, returns HTML with syntax-highlighted spans.
 */

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

export function highlightJSON(json: string): string {
  const escaped = escapeHtml(json);

  return escaped.replace(
    /("(?:[^"\\]|\\.)*")\s*(:)|("(?:[^"\\]|\\.)*")|((?:-?\d+\.?\d*(?:[eE][+-]?\d+)?))|(\btrue\b|\bfalse\b|\bnull\b)|([{}[\]:,])/g,
    (
      _match: string,
      key: string | undefined,
      colon: string | undefined,
      str: string | undefined,
      num: string | undefined,
      boo: string | undefined,
      pu: string | undefined,
    ) => {
      if (key !== undefined && colon !== undefined) {
        return `<span class="key">${key}</span><span class="pu">${colon}</span>`;
      }
      if (str !== undefined) {
        return `<span class="str">${str}</span>`;
      }
      if (num !== undefined) {
        return `<span class="num">${num}</span>`;
      }
      if (boo !== undefined) {
        return `<span class="boo">${boo}</span>`;
      }
      if (pu !== undefined) {
        return `<span class="pu">${pu}</span>`;
      }
      return _match;
    },
  );
}
