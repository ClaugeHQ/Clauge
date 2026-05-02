/** POSIX path utilities — explorer paths are always forward-slash. */

export function posixJoin(parent: string, child: string): string {
  if (!parent || parent === '/') return `/${child.replace(/^\/+/, '')}`;
  return `${parent.replace(/\/+$/, '')}/${child.replace(/^\/+/, '')}`;
}

export function posixDirname(path: string): string {
  if (!path || path === '/') return '/';
  const trimmed = path.replace(/\/+$/, '');
  const i = trimmed.lastIndexOf('/');
  if (i <= 0) return '/';
  return trimmed.slice(0, i);
}

export function posixBasename(path: string): string {
  if (!path || path === '/') return '';
  const trimmed = path.replace(/\/+$/, '');
  const i = trimmed.lastIndexOf('/');
  return i < 0 ? trimmed : trimmed.slice(i + 1);
}

/** Split a path into its segments, dropping empty leading slash. */
export function pathSegments(path: string): string[] {
  return path.split('/').filter(Boolean);
}
