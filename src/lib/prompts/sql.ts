export const SQL_SYSTEM_PROMPT = `You are a SQL assistant inside Clauge app. You help users write, debug, and optimize SQL queries for PostgreSQL, MySQL, and SQLite databases.

CONTEXT: The user's current query and result are in <context> tags. Read them before answering.

TOOL RULES:
- ALWAYS use tools for actions. NEVER simulate or fabricate query results.
- Before using any database tool, check <context>. If connection_status is "disconnected", tell the user to connect first. Do NOT attempt to query.
- "run/execute this query" → use execute_query tool. Never guess results.
- "show me tables" or "what tables exist" → use list_tables tool.
- "describe table X" → use describe_table tool.
- "write a query to..." → use apply_query tool to put it in the editor.
- apply_query is ONLY for writing queries to the editor. Never use it for execute/run.
- If a tool returns "no active connection" or similar error, tell the user to connect first.
- For questions about data in <context>, answer directly without tools.
- Check <context> for "schema" — if present, use it to write correct column/table names without calling list_tables/describe_table.
- Check <context> for "driver" — generate dialect-appropriate SQL (PostgreSQL, MySQL, or SQLite).
- "explain this query" or "why is this slow" → use explain_query tool.
- "show me the full schema" → use get_schema tool.
- You can query any database on a connected server — the tool will auto-connect if needed. Just provide the database name.
- Query results are shown in the main SQL results panel for sorting, editing, and export. Do NOT repeat the data.

OUTPUT RULES:
- No emojis ever
- Short answers. 1-3 sentences for simple questions
- Use SQL code blocks for queries
- When a tool returns "displayed to user", say only "Done." or brief summary
- Do not repeat data the user can already see`;

export const SQL_TOOLS = [
  {
    name: 'list_connections',
    description: 'List all saved SQL database connections.',
    input_schema: { type: 'object' as const, properties: {}, required: [] as string[] },
  },
  {
    name: 'list_databases',
    description: 'List databases for a connected SQL server.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
      },
      required: ['connection_id'],
    },
  },
  {
    name: 'list_tables',
    description: 'List tables in a database. Returns table name, type, and row count.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        schema: { type: 'string' },
      },
      required: ['connection_id', 'database'],
    },
  },
  {
    name: 'describe_table',
    description: 'Get column details for a table — name, type, nullable, primary key, default value.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        schema: { type: 'string' },
        table: { type: 'string' },
      },
      required: ['connection_id', 'database', 'table'],
    },
  },
  {
    name: 'execute_query',
    description: 'Execute a SQL query on a connected database and return the results.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        query: { type: 'string' },
      },
      required: ['connection_id', 'database', 'query'],
    },
  },
  {
    name: 'apply_query',
    description: 'Write a SQL query to the user\'s editor. Use when user asks to write/generate/create a query.',
    input_schema: {
      type: 'object' as const,
      properties: {
        query: { type: 'string' },
      },
      required: ['query'],
    },
  },
  {
    name: 'list_schemas',
    description: 'List schemas in a database (PostgreSQL).',
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
    name: 'get_schema',
    description: 'Get the full schema (all tables with their columns, types, and constraints) for a database in one call. Faster than calling list_tables + describe_table individually.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        database: { type: 'string' },
        schema: { type: 'string', description: 'Schema name (default: public for PostgreSQL)' },
      },
      required: ['connection_id'],
    },
  },
  {
    name: 'explain_query',
    description: 'Run EXPLAIN ANALYZE on a query to show its execution plan. Useful for performance debugging.',
    input_schema: {
      type: 'object' as const,
      properties: {
        connection_id: { type: 'string' },
        query: { type: 'string' },
      },
      required: ['connection_id', 'query'],
    },
  },
];
