import { writable } from 'svelte/store';
import type { Environment, EnvVariable } from '$lib/types';
import * as cmd from '$lib/commands';

export const environments = writable<Environment[]>([]);

// Persist active env selection
const savedEnvId = typeof localStorage !== 'undefined' ? localStorage.getItem('clauge_active_env_id') : null;
export const activeEnvId = writable<string | null>(savedEnvId);

export async function loadEnvironments() {
  try {
    const envs = await cmd.listEnvironments();
    environments.set(envs);
    // Read current activeEnvId from localStorage (not the stale module-level snapshot)
    const current = typeof localStorage !== 'undefined' ? localStorage.getItem('clauge_active_env_id') : null;
    const currentExists = current && envs.some(e => e.id === current);
    if (!currentExists && envs.length > 0) {
      const def = envs.find(e => e.isDefault === 1);
      if (def) setActiveEnv(def.id);
      else setActiveEnv(envs[0].id);
    } else if (!currentExists && envs.length === 0) {
      activeEnvId.set(null);
      if (typeof localStorage !== 'undefined') {
        localStorage.removeItem('clauge_active_env_id');
      }
    }
  } catch (err) {
    console.error('Failed to load environments:', err);
  }
}

export async function createEnvironment(name: string, color: string) {
  const env = await cmd.createEnvironment(name, color);
  environments.update(e => [...e, env]);
  // Auto-activate if it's the first (and now default) environment
  if (env.isDefault === 1) {
    setActiveEnv(env.id);
  }
  return env;
}

export async function updateEnvironment(id: string, name: string, color: string) {
  const env = await cmd.updateEnvironment(id, name, color);
  environments.update(e => e.map(x => x.id === id ? env : x));
}

export async function deleteEnvironment(id: string) {
  await cmd.deleteEnvironment(id);
  environments.update(e => e.filter(x => x.id !== id));
  // Clear activeEnvId if the deleted env was the active one
  const currentActive = typeof localStorage !== 'undefined' ? localStorage.getItem('clauge_active_env_id') : null;
  if (currentActive === id) {
    activeEnvId.set(null);
    if (typeof localStorage !== 'undefined') {
      localStorage.removeItem('clauge_active_env_id');
    }
  }
  // Remove all per-request overrides pointing to the deleted env (revert to global)
  if (typeof localStorage !== 'undefined') {
    const overridesRaw = localStorage.getItem('clauge_request_env_overrides');
    if (overridesRaw) {
      try {
        const overrides = JSON.parse(overridesRaw);
        const cleaned: Record<string, string> = {};
        for (const [key, val] of Object.entries(overrides)) {
          if (val !== id) cleaned[key] = val as string;
        }
        localStorage.setItem('clauge_request_env_overrides', JSON.stringify(cleaned));
        // Also need to import and update the store
        const { requestEnvOverrides } = await import('$lib/stores/collections');
        requestEnvOverrides.set(cleaned);
      } catch {}
    }
  }
}

export async function setDefaultEnv(id: string) {
  await cmd.setDefaultEnvironment(id);
  activeEnvId.set(id);
  await loadEnvironments();
}

export async function setActiveEnv(id: string) {
  activeEnvId.set(id);
  if (typeof localStorage !== 'undefined') {
    localStorage.setItem('clauge_active_env_id', id);
  }
}

export async function loadEnvVariables(envId: string): Promise<EnvVariable[]> {
  return cmd.listEnvVariables(envId);
}

export async function setEnvVariable(envId: string, key: string, value: string, isSecret: number) {
  return cmd.setEnvVariable(envId, key, value, isSecret);
}

export async function updateEnvVariable(id: string, key: string, value: string, isSecret: number) {
  return cmd.updateEnvVariable(id, key, value, isSecret);
}

export async function deleteEnvVariable(id: string) {
  return cmd.deleteEnvVariable(id);
}

export function getEffectiveEnvId(
  requestOrTabId: string | null,
  overrides: Record<string, string>,
  globalEnvId: string | null,
): string | null {
  if (requestOrTabId && overrides[requestOrTabId]) {
    return overrides[requestOrTabId];
  }
  return globalEnvId;
}
