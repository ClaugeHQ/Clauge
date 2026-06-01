import { get } from 'svelte/store';
import { agentTerminalMap } from '$lib/modes/agent/stores';

/**
 * Move the xterm container for `sessionId` into `slot`. No-op if there is
 * no entry yet (the session hasn't been activated in AgentPanel and so the
 * xterm hasn't been created).
 *
 * Uses native `appendChild` semantics: if the container is already parented
 * elsewhere, the browser silently removes it from the old parent first.
 * After moving, the FitAddon refits to the new container's dimensions.
 */
export function attachAgentTerminal(sessionId: string, slot: HTMLElement): void {
  const entry = get(agentTerminalMap).get(sessionId);
  if (!entry) return;
  slot.appendChild(entry.container);
  // Lazy term.open: xterm's `terminal.element` is set after the first
  // `open(container)` call. For sessions that were created but never
  // mounted (Canvas-only view of an inactive Agent session), term.element
  // is undefined and the container is empty. Open here, with the container
  // already in a live slot, to avoid the WebGL-on-detached-canvas issue.
  try {
    if (!entry.term?.element) {
      entry.term.open(entry.container);
    }
    entry.fitAddon?.fit();
  } catch (err) {
    console.error('[canvas] attachAgentTerminal fit/open failed:', err);
  }
}

/**
 * Move the xterm container OUT of `slot` if it currently sits there.
 * Safe to call when the container is already elsewhere. Does NOT destroy
 * the xterm or detach the PTY — the entry stays alive in agentTerminalMap.
 */
export function detachAgentTerminal(sessionId: string, slot: HTMLElement): void {
  const entry = get(agentTerminalMap).get(sessionId);
  if (!entry) return;
  if (entry.container.parentElement === slot) {
    slot.removeChild(entry.container);
  }
}

/**
 * Return the list of currently-open agent sessions (entries whose PTY has
 * been spawned, i.e. terminalId is non-null).
 * Used by the Canvas adapter's `listOpenTabs`.
 */
export function listOpenAgentTerminals(): { id: string; title: string }[] {
  const m = get(agentTerminalMap);
  const out: { id: string; title: string }[] = [];
  for (const [sessionId, entry] of m) {
    // Include any entry with a created xterm container, even if the PTY
    // hasn't connected yet (terminalId == null). The xterm will mount;
    // once the PTY connects, output flows into it.
    if (entry?.container) {
      out.push({ id: sessionId, title: sessionId });
    }
  }
  return out;
}
