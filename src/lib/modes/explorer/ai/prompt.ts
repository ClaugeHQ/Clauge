/** AI assistance prompt + tool descriptors for Explorer mode. */

export const EXPLORER_SYSTEM_PROMPT = `You are the Clauge Explorer assistant — you help users browse and manage remote storage (SFTP, FTP, S3-compatible, Azure Blob) through natural-language commands.

You have access to file-system tools. The currently-open Explorer tab is identified by a \`tabKey\` value that the user's app provides automatically. When calling a tool, pass the user's current tabKey through unchanged.

Path conventions:
- Always use forward-slash POSIX paths.
- For S3 / Azure Blob backends, paths look like \`/<bucket-or-container>/<key>\`. The first path segment IS the bucket or container.
- For SFTP / FTP, paths look like ordinary filesystem paths (\`/var/log\`, \`/home/user\`).

Tool semantics:
- Read tools (\`fs_list\`, \`fs_stat\`, \`fs_read\`, \`fs_search\`, \`fs_get_url\`) execute immediately.
- Write tools (\`fs_write\`, \`fs_delete\`, \`fs_mkdir\`, \`fs_rename\`, \`fs_upload_local\`, \`fs_download\`) require the user to confirm before they run; explain what you're about to do and wait for their approval.

Be precise. Don't speculate about file contents you haven't read. When listing many entries, group similar items rather than dumping the full list.`;

export const EXPLORER_TOOLS = [
  {
    name: 'fs_list',
    description: 'List entries in a remote directory.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string', description: 'Active Explorer tab identifier (provided by the app).' },
        path: { type: 'string', description: 'POSIX path to list. Defaults to "/" if omitted.' },
      },
      required: ['tabKey'],
    },
  },
  {
    name: 'fs_stat',
    description: 'Get metadata (size, modified time, permissions, mime type) for a single remote path.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        path: { type: 'string' },
      },
      required: ['tabKey', 'path'],
    },
  },
  {
    name: 'fs_read',
    description: 'Read the contents of a remote file. Defaults to a 64 KB cap; pass `maxBytes` to override.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        path: { type: 'string' },
        maxBytes: { type: 'number' },
      },
      required: ['tabKey', 'path'],
    },
  },
  {
    name: 'fs_search',
    description: 'Recursively search under a prefix for files whose name matches a POSIX-style glob (* and ? supported). Capped at 5000 entries / depth 8.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        prefix: { type: 'string' },
        glob: { type: 'string' },
      },
      required: ['tabKey', 'prefix', 'glob'],
    },
  },
  {
    name: 'fs_get_url',
    description: 'Generate a temporary signed URL for an object (S3 / Azure Blob backends only — returns a clarifying message for SFTP / FTP).',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        path: { type: 'string' },
        ttlSecs: { type: 'number' },
      },
      required: ['tabKey', 'path'],
    },
  },
  {
    name: 'fs_write',
    description: 'Write text content to a remote path. Requires user confirmation.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        path: { type: 'string' },
        content: { type: 'string', description: 'UTF-8 text to write.' },
      },
      required: ['tabKey', 'path', 'content'],
    },
  },
  {
    name: 'fs_delete',
    description: 'Delete one or more remote paths. Requires user confirmation.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        paths: { type: 'array', items: { type: 'string' } },
      },
      required: ['tabKey', 'paths'],
    },
  },
  {
    name: 'fs_mkdir',
    description: 'Create a remote directory. Requires user confirmation.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        path: { type: 'string' },
      },
      required: ['tabKey', 'path'],
    },
  },
  {
    name: 'fs_rename',
    description: 'Rename / move a remote path. Requires user confirmation.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        from: { type: 'string' },
        to: { type: 'string' },
      },
      required: ['tabKey', 'from', 'to'],
    },
  },
  {
    name: 'fs_upload_local',
    description: 'Upload a local file to a remote path. Requires user confirmation.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        localPath: { type: 'string' },
        remotePath: { type: 'string' },
      },
      required: ['tabKey', 'localPath', 'remotePath'],
    },
  },
  {
    name: 'fs_download',
    description: 'Download a remote file to a local path. Requires user confirmation.',
    input_schema: {
      type: 'object',
      properties: {
        tabKey: { type: 'string' },
        remotePath: { type: 'string' },
        localPath: { type: 'string' },
      },
      required: ['tabKey', 'remotePath', 'localPath'],
    },
  },
] as const;
