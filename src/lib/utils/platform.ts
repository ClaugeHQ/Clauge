import { platform as tauriPlatform } from '@tauri-apps/plugin-os';

export type Platform = 'macos' | 'windows' | 'linux';

let cached: Platform | null = null;

export function platform(): Platform {
	if (cached) return cached;
	const p = tauriPlatform();
	cached = p === 'macos' || p === 'windows' || p === 'linux' ? p : 'linux';
	return cached;
}

export const isMac = (): boolean => platform() === 'macos';
export const isWindows = (): boolean => platform() === 'windows';
export const isLinux = (): boolean => platform() === 'linux';

/** Modifier key label for shortcuts UI ("Cmd" on macOS, "Ctrl" elsewhere). */
export const mod = (): string => (isMac() ? 'Cmd' : 'Ctrl');

/** True if a keyboard event matches the platform's primary modifier. */
export function modKey(e: KeyboardEvent | MouseEvent): boolean {
	return isMac() ? e.metaKey : e.ctrlKey;
}
