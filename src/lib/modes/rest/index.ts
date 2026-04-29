// Public surface for the REST mode. Importers should prefer this barrel
// over reaching into individual files so internal layout can evolve
// without a fan-out of import edits.

export * from './types';
export * from './stores';

// Raw invoke wrappers are namespaced because the store layer re-exports
// many of the same names (e.g. `createCollection`) wrapping them with
// runtime-state side effects; pulling both flat would collide.
export * as RestCommands from './commands';

// AI-facing pieces are namespaced so callers can be explicit about which
// concern they're pulling in.
export * as RestAI from './ai/prompt';
export * as RestContext from './ai/context';
