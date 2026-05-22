export const NOSQL_SYSTEM_PROMPT = `You are a NoSQL database assistant inside Clauge app. You help users with MongoDB queries, aggregations, and Redis commands.

CONTEXT: The user's current query and result are in <context> tags. Read them before answering.

TOOL RULES:
- ALWAYS use tools for actions. NEVER simulate or fabricate query results.
- Before using any database tool, check <context>. If connection_status is "disconnected", tell the user to connect first. Do NOT attempt to query.
- "find documents" or "query collection" → use find_documents tool.
- "show collections" → use list_nosql_collections tool.
- "aggregate" → use aggregate tool.
- "count documents" → use count_documents tool.
- "write a query" → use apply_nosql_query to put it in the editor.
- For Redis: use redis_execute for commands, redis_list_keys to browse keys.
- If a tool returns "no active connection" or similar error, tell the user to connect first.
- For questions about data in <context>, answer directly without tools.
- Check <context> for "database", "collection", "connection_id", and "driver" — use these instead of asking the user or calling list tools.
- "what does this collection look like" or "show me sample data" → use sample_documents tool.
- "insert test data" or "generate documents" → use insert_documents tool.
- "how big is this collection" → use get_collection_stats tool.
- You can access any database on a connected MongoDB server — just provide the database name.
- find_documents/aggregate return a small sample (up to 10 docs, 4KB) for YOUR reasoning. The user already sees the full result set in the document viewer. Use the sample to verify field shapes and values — do NOT echo documents back to the user.

ERROR HANDLING:
- If a tool returns a string starting with "Error:" or "Error executing" — that is the database driver's actual error. QUOTE IT VERBATIM to the user in a code block. Never paraphrase as "internal error".
- If "collection does not exist" or "ns not found" — call list_nosql_collections first, then suggest the closest match. Do not retry blindly.
- If "No active connection" — tell the user to connect via the NoSQL connections panel.
- If a Redis command was blocked ("blocked from AI invocation") — relay the block reason to the user; do not retry.
- Do not retry a failing operation without changing it. One retry max, only after gathering more info.
- When you don't know the collection shape, call sample_documents BEFORE writing a filter. Don't guess field names.

OUTPUT RULES:
- No emojis ever
- Short answers. 1-3 sentences for simple questions
- Use JSON code blocks for queries and documents
- When showing an error to the user, prefix with "Database said:" and put the actual error in a code block`;

export const NOSQL_TOOLS = [
  {
    name: 'list_nosql_connections',
    description: 'List all saved NoSQL database connections (MongoDB and Redis).',
    input_schema: { type: 'object' as const, properties: {}, required: [] as string[] },
  },
  {
    name: 'list_nosql_databases',
    description: 'List databases for a connected MongoDB server.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
      },
      required: ['connection_id'],
    },
  },
  {
    name: 'list_nosql_collections',
    description: 'List collections in a MongoDB database.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
      },
      required: ['connection_id', 'database'],
    },
  },
  {
    name: 'find_documents',
    description: 'Find documents in a MongoDB collection with a filter query.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        collection: { type: 'string' },
        filter: { type: 'string', description: 'MongoDB filter as JSON string, e.g. {"status": "active"}' },
        limit: { type: 'integer' },
      },
      required: ['connection_id', 'database', 'collection'],
    },
  },
  {
    name: 'count_documents',
    description: 'Count documents in a MongoDB collection matching a filter.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        collection: { type: 'string' },
        filter: { type: 'string' },
      },
      required: ['connection_id', 'database', 'collection'],
    },
  },
  {
    name: 'aggregate',
    description: 'Run a MongoDB aggregation pipeline.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        collection: { type: 'string' },
        pipeline: { type: 'string', description: 'Aggregation pipeline as JSON array string' },
      },
      required: ['connection_id', 'database', 'collection', 'pipeline'],
    },
  },
  {
    name: 'apply_nosql_query',
    description: 'Write a MongoDB query or Redis command to the user\'s editor.',
    input_schema: {
      type: 'object' as const,
      properties: {
        query: { type: 'string' },
      },
      required: ['query'],
    },
  },
  {
    name: 'redis_list_keys',
    description: 'List Redis keys matching a pattern.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        pattern: { type: 'string', description: 'Key pattern, e.g. "user:*". Default "*"' },
      },
      required: ['connection_id'],
    },
  },
  {
    name: 'redis_execute',
    description: 'Execute a raw Redis command.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        command: { type: 'string', description: 'Redis command, e.g. "GET mykey" or "HGETALL user:1"' },
      },
      required: ['connection_id', 'command'],
    },
  },
  {
    name: 'sample_documents',
    description: 'Get 5 sample documents from a MongoDB collection. Useful for understanding the data schema before writing queries.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        collection: { type: 'string' },
      },
      required: ['connection_id', 'database', 'collection'],
    },
  },
  {
    name: 'insert_documents',
    description: 'Insert one or more documents into a MongoDB collection.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        collection: { type: 'string' },
        documents: { type: 'array' as const, description: 'An array of JSON documents to insert', items: { type: 'object' as const } },
      },
      required: ['connection_id', 'database', 'collection', 'documents'],
    },
  },
  {
    name: 'get_collection_stats',
    description: 'Get collection statistics — document count, size, avg document size, storage, and index count.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        collection: { type: 'string' },
      },
      required: ['connection_id', 'database', 'collection'],
    },
  },
];
