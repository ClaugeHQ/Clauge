// Shared markdown rendering for card description + comments (ticket and
// coworker sections). Images are never auto-loaded: every <img> becomes a
// click-to-reveal chip so screenshot-heavy tickets/threads don't slam into
// view at full size. `handleImageToggleClick` is the delegated click handler
// that reveals/collapses an image; wire it to the container's onclick.

import { marked } from 'marked';
import DOMPurify from 'dompurify';

/** Render markdown to HTML with images replaced by click-to-reveal chips.
 *  The chip's `data-src`/`data-alt` are consumed by handleImageToggleClick.
 *  Output is sanitized — card descriptions and (crucially) fetched
 *  GitHub/GitLab issue comments are untrusted content, so raw HTML in the
 *  markdown must never reach the DOM unscrubbed. */
export function renderCardMarkdown(text: string): string {
  if (!text || !text.trim()) return '';
  const raw = marked.parse(text, { async: false }) as string;
  // Allow data: image URIs (pasted screenshots) through the sanitizer.
  let html = DOMPurify.sanitize(raw, { ADD_DATA_URI_TAGS: ['img'] });
  html = html.replace(/<img\s+([^>]*?)\/?>/gi, (_match, attrs: string) => {
    const srcMatch = attrs.match(/src=["']([^"']+)["']/i);
    const altMatch = attrs.match(/alt=["']([^"']*)["']/i);
    const src = srcMatch ? srcMatch[1] : '';
    const alt = (altMatch ? altMatch[1] : '').trim() || 'image';
    if (!src) return '';
    const safeSrc = src.replace(/"/g, '&quot;');
    const safeAlt = alt.replace(/</g, '&lt;');
    return `<button type="button" class="th-img-toggle" data-src="${safeSrc}" data-alt="${safeAlt}">📎 ${safeAlt} <span class="th-img-hint">· click to view</span></button>`;
  });
  return html;
}

/** Delegated click handler for revealing/collapsing an image chip.
 *  Returns true when it handled an image toggle (so callers can skip their
 *  own click behavior — e.g. entering description edit mode). */
export function handleImageToggleClick(e: MouseEvent): boolean {
  const target = e.target as HTMLElement | null;
  const btn = target?.closest('.th-img-toggle, .th-img-revealed') as HTMLElement | null;
  if (!btn) return false;
  e.preventDefault();
  e.stopPropagation();
  const src = btn.dataset.src ?? '';
  const alt = btn.dataset.alt ?? '';
  // Build via DOM APIs (properties/textContent), never innerHTML with
  // interpolated values — so a hostile src/alt can't inject markup.
  if (btn.classList.contains('th-img-toggle')) {
    const wrap = document.createElement('span');
    wrap.className = 'th-img-revealed';
    wrap.dataset.src = src;
    wrap.dataset.alt = alt;
    const img = document.createElement('img');
    img.src = src;
    img.alt = alt;
    const collapse = document.createElement('span');
    collapse.className = 'th-img-collapse';
    collapse.textContent = 'collapse';
    wrap.append(img, collapse);
    btn.replaceWith(wrap);
  } else {
    const ph = document.createElement('button');
    ph.type = 'button';
    ph.className = 'th-img-toggle';
    ph.dataset.src = src;
    ph.dataset.alt = alt;
    const hint = document.createElement('span');
    hint.className = 'th-img-hint';
    hint.textContent = '· click to view';
    ph.append(document.createTextNode(`📎 ${alt} `), hint);
    btn.replaceWith(ph);
  }
  return true;
}
