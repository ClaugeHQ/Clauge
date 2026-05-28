import { writable } from 'svelte/store';
import type { AppearanceConfig } from '$lib/types';
import * as cmd from '$lib/commands';
import { DEFAULT_ACCENT_COLOR } from '$lib/shared/constants/colors';

export const settings = writable<Record<string, string>>({});
export const appearance = writable<AppearanceConfig>({
  theme: 'dark-glass',
  accentColor: DEFAULT_ACCENT_COLOR,
  terminalFontFamily: '',
});

export async function loadSettings() {
  try {
    const all = await cmd.getAllSettings();
    settings.set(all);
  } catch (err) {
    console.error('Failed to load settings:', err);
  }
}

export async function setSetting(key: string, value: string) {
  await cmd.setSetting(key, value);
  settings.update(s => ({ ...s, [key]: value }));
}

export async function loadAppearance() {
  try {
    const config = await cmd.getAppearance();
    appearance.set(config);
  } catch (err) {
    console.error('Failed to load appearance:', err);
  }
}

export async function saveAppearance(config: AppearanceConfig) {
  await cmd.setAppearance(config);
  appearance.set(config);
}
