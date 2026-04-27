// Coordination layer between AIPanel (which receives tool_pending events
// from Rust) and SshPanel (which owns the xterm + PTY writer). Avoids
// circular imports by using a tiny pub/sub indirection.
//
// Flow:
//   AIPanel → showSshExecuteConfirmModal(...)
//   user approves
//   AIPanel → executeAndCaptureOnSsh(profileId, command)
//     → emits ssh:execute-capture-request to SshPanel
//   SshPanel writes command to xterm, captures output until prompt or timeout
//     → resolveSshCapture(requestId, output)
//   AIPanel: promise resolves, redacts, calls ai_resolve_pending_tool

const pendingCaptures = new Map<string, (output: string) => void>();

export interface SshCaptureRequest {
  requestId: string;
  profileId: string;
  command: string;
}

/**
 * Ask the SshPanel to write a command and capture its output.
 * Returns the raw (un-redacted) terminal output once captured.
 * Caller is responsible for running `redactSecrets` before forwarding to AI.
 */
export function executeAndCaptureOnSsh(profileId: string, command: string): Promise<string> {
  const requestId = crypto.randomUUID();
  const promise = new Promise<string>((resolve) => {
    pendingCaptures.set(requestId, resolve);
  });
  const detail: SshCaptureRequest = { requestId, profileId, command };
  window.dispatchEvent(new CustomEvent('ssh:execute-capture-request', { detail }));
  return promise;
}

/** Called by SshPanel when capture has completed (success or timeout). */
export function resolveSshCapture(requestId: string, output: string): void {
  const fn = pendingCaptures.get(requestId);
  if (fn) {
    pendingCaptures.delete(requestId);
    fn(output);
  }
}

/** Reject all pending captures — used when an SSH tab closes mid-capture. */
export function rejectAllSshCaptures(reason: string): void {
  for (const [id, resolve] of pendingCaptures) {
    resolve(`[ERROR] ${reason}`);
    pendingCaptures.delete(id);
  }
}
