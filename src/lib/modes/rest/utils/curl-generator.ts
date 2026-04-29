import type { RequestWithDetails } from '$lib/types';

export function generateCurl(request: RequestWithDetails, resolvedUrl?: string): string {
  const url = resolvedUrl || request.url;
  let cmd = `curl -X ${request.method}`;
  cmd += ` '${url}'`;

  for (const h of request.headers.filter(h => h.enabled)) {
    cmd += ` \\\n  -H '${h.key}: ${h.value}'`;
  }

  if (request.body && ['POST', 'PUT', 'PATCH'].includes(request.method)) {
    cmd += ` \\\n  -d '${request.body.replace(/'/g, "'\\''")}'`;
  }

  return cmd;
}
