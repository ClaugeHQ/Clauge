/**
 * Friendly display name for a cloud-sync kind id — keeps UI copy in the
 * user's language, not the protocol's. Shared by the conflict resolver
 * and the Account settings sync panels.
 */
export function kindLabel(kind: string): string {
  switch (kind) {
    case 'rest':             return 'REST collections';
    case 'sql':              return 'SQL connections';
    case 'nosql':            return 'NoSQL connections';
    case 'agent':            return 'Agent contexts';
    case 'ssh':              return 'SSH profiles';
    case 'explorer':         return 'Explorer connections';
    case 'coworkers':        return 'Workspace coworkers';
    case 'workspace_notes':  return 'Workspace notes';
    default:                 return kind;
  }
}
