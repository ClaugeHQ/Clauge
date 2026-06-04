import {
  viewport,
  interactionState,
  flushViewportSoon,
} from '$lib/modes/canvas/stores/canvasStore';

const RIGHT_BUTTON = 2;
const MIDDLE_BUTTON = 1;

/**
 * Svelte action: makes the host element pan-aware.
 * Gestures: right-click drag, middle-click drag, space+left-drag, wheel (without ctrl).
 * Sets body class `cv-interacting` during active gesture so children can suppress fits.
 */
export function pannable(node: HTMLElement) {
  let dragging = false;
  let lastX = 0;
  let lastY = 0;
  let pointerId: number | null = null;
  let spaceHeld = false;

  function onPointerDown(e: PointerEvent) {
    const isPanGesture =
      e.button === RIGHT_BUTTON ||
      e.button === MIDDLE_BUTTON ||
      (e.button === 0 && spaceHeld);
    if (!isPanGesture) return;
    e.preventDefault();
    dragging = true;
    lastX = e.clientX;
    lastY = e.clientY;
    pointerId = e.pointerId;
    node.setPointerCapture(e.pointerId);
    interactionState.set('panning');
    document.body.classList.add('cv-interacting');
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging || e.pointerId !== pointerId) return;
    const dx = e.clientX - lastX;
    const dy = e.clientY - lastY;
    lastX = e.clientX;
    lastY = e.clientY;
    viewport.update((v) => ({ ...v, offsetX: v.offsetX + dx, offsetY: v.offsetY + dy }));
  }

  function endPan(e: PointerEvent) {
    if (!dragging || e.pointerId !== pointerId) return;
    dragging = false;
    pointerId = null;
    try {
      node.releasePointerCapture(e.pointerId);
    } catch {
      // Capture may already be released by browser.
    }
    interactionState.set('idle');
    document.body.classList.remove('cv-interacting');
    flushViewportSoon();
  }

  function onWheel(e: WheelEvent) {
    if (e.ctrlKey || e.metaKey) return; // ctrl/cmd reserved for zoom action.
    // If the wheel happened inside a tile, let the tile's scrollable
    // descendants handle it. Pan-the-canvas is only for wheel over the
    // empty canvas background.
    const target = e.target as HTMLElement | null;
    if (target?.closest('.cv-tile')) return;
    e.preventDefault();
    viewport.update((v) => ({
      ...v,
      offsetX: v.offsetX - e.deltaX,
      offsetY: v.offsetY - e.deltaY,
    }));
    flushViewportSoon();
  }

  function isTypingTarget(t: EventTarget | null): boolean {
    if (!(t instanceof HTMLElement)) return false;
    return t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable;
  }

  function onContextMenu(e: MouseEvent) {
    // Right-button is our pan gesture; suppress the OS context menu that
    // would otherwise pop at button release at the end of every drag.
    e.preventDefault();
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.code === 'Space' && !spaceHeld && !isTypingTarget(e.target)) {
      spaceHeld = true;
      node.style.cursor = 'grab';
    }
  }

  function onKeyUp(e: KeyboardEvent) {
    if (e.code === 'Space') {
      spaceHeld = false;
      if (!dragging) node.style.cursor = '';
    }
  }

  node.addEventListener('pointerdown', onPointerDown);
  node.addEventListener('pointermove', onPointerMove);
  node.addEventListener('pointerup', endPan);
  node.addEventListener('pointercancel', endPan);
  node.addEventListener('wheel', onWheel, { passive: false });
  node.addEventListener('contextmenu', onContextMenu);
  window.addEventListener('keydown', onKeyDown);
  window.addEventListener('keyup', onKeyUp);

  return {
    destroy() {
      node.removeEventListener('pointerdown', onPointerDown);
      node.removeEventListener('pointermove', onPointerMove);
      node.removeEventListener('pointerup', endPan);
      node.removeEventListener('pointercancel', endPan);
      node.removeEventListener('wheel', onWheel);
      node.removeEventListener('contextmenu', onContextMenu);
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('keyup', onKeyUp);
    },
  };
}
