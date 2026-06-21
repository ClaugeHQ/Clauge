// Image attachment for card text fields. Follows the codebase convention
// (Notes/Milkdown editor): images are embedded as base64 data URIs so they
// persist in the same row as the text — no asset-protocol config, and the
// shared card markdown renderer already shows <img src="data:…"> via its
// click-to-reveal chip.
//
// Note: Tauri intercepts OS file *drops*, so dragging from Finder may not
// fire here — paste (⌘V a screenshot) and the file-picker button are the
// reliable paths; drop is wired anyway for platforms where it works.

const MAX_BYTES = 10 * 1024 * 1024; // 10 MB guard

export function fileToDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error ?? new Error('image read failed'));
    reader.readAsDataURL(file);
  });
}

/** Build the markdown for an image file, or null if it's not a usable image. */
export async function imageMarkdown(file: File): Promise<string | null> {
  if (!file.type.startsWith('image/')) return null;
  if (file.size > MAX_BYTES) throw new Error('Image is larger than 10 MB');
  const url = await fileToDataUrl(file);
  const name = (file.name || 'image').replace(/[[\]]/g, '');
  return `\n![${name}](${url})\n`;
}

/** Insert text at the textarea caret and return the new value. Caret is
 *  moved to the end of the inserted text on the next frame. */
export function insertAtCaret(el: HTMLTextAreaElement, current: string, snippet: string): string {
  const start = el.selectionStart ?? current.length;
  const end = el.selectionEnd ?? current.length;
  const next = current.slice(0, start) + snippet + current.slice(end);
  requestAnimationFrame(() => {
    try {
      el.focus();
      el.selectionStart = el.selectionEnd = start + snippet.length;
    } catch {
      /* ignore */
    }
  });
  return next;
}

/** Open a native image file picker; resolves to the chosen File or null. */
export function pickImageFile(): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/*';
    input.style.display = 'none';
    input.onchange = () => {
      resolve(input.files?.[0] ?? null);
      input.remove();
    };
    document.body.appendChild(input);
    input.click();
  });
}

/** Svelte action: paste/drop an image into a textarea. `insert` receives
 *  the image markdown and is responsible for splicing it into the value. */
export function imagePaste(
  node: HTMLTextAreaElement,
  insert: (markdown: string) => void,
) {
  let cb = insert;
  async function fromFile(file: File) {
    try {
      const md = await imageMarkdown(file);
      if (md) cb(md);
    } catch {
      /* surfaced elsewhere; ignore here */
    }
  }
  function onPaste(e: ClipboardEvent) {
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const it of items) {
      if (it.kind === 'file' && it.type.startsWith('image/')) {
        const f = it.getAsFile();
        if (f) {
          e.preventDefault();
          fromFile(f);
          return;
        }
      }
    }
  }
  function onDrop(e: DragEvent) {
    const files = e.dataTransfer?.files;
    if (!files?.length) return;
    for (const f of files) {
      if (f.type.startsWith('image/')) {
        e.preventDefault();
        fromFile(f);
        return;
      }
    }
  }
  function onDragOver(e: DragEvent) {
    if (Array.from(e.dataTransfer?.items ?? []).some((i) => i.kind === 'file')) {
      e.preventDefault();
    }
  }
  node.addEventListener('paste', onPaste);
  node.addEventListener('drop', onDrop);
  node.addEventListener('dragover', onDragOver);
  return {
    update(next: (markdown: string) => void) {
      cb = next;
    },
    destroy() {
      node.removeEventListener('paste', onPaste);
      node.removeEventListener('drop', onDrop);
      node.removeEventListener('dragover', onDragOver);
    },
  };
}
