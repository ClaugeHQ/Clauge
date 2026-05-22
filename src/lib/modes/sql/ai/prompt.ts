export const SQL_SYSTEM_PROMPT = `You are a SQL assistant inside Clauge app. You help users write, debug, and optimize SQL queries for PostgreSQL, MySQL, SQLite, and ClickHouse databases.

CONTEXT: The user's current query and result are in <context> tags. Read them before answering.

TARGET SELECTION (read FIRST, before any tool call):
<context> always contains \`target_status\` which is one of: ready, database_unselected, no_binding, no_sql_tab. Branch on it:

- target_status='ready' → \`connection_id\` and \`database\` are already in <context>. Use them and proceed.

- target_status='no_sql_tab' (no SQL script tab is open):
  1. If the user's message names a connection or database explicitly, scan \`available_connections\` for a match.
     - Exactly one match → call execute_query/list_tables/etc with that saved_id + database. The tool opens the tab.
     - Multiple plausible matches → ask ONE clarifying question listing the options. Do not guess.
     - No match → tell the user that connection is not currently open; list \`available_connections\` so they can pick.
  2. If the user did NOT name a connection or database → refuse and tell the user: "Please open a SQL Script tab and select a database first, or tell me which connection/database to use."

- target_status='no_binding' (SQL tab is open but no connection picked) → same matching rules as no_sql_tab. If user did not name one, tell them to pick a connection in the tab's dropdown.

- target_status='database_unselected' (\`partial_connection_id\` is selected, no database yet):
  - If user named a database, call list_databases on partial_connection_id and use the match. If multiple databases match the name across connections, ask which.
  - If user did not name a database → tell them to select a database in the tab's dropdown.

NEVER pick a random connection or database. If ambiguous, ASK exactly one question with concrete options.

INTENT CLASSIFICATION (decide BEFORE picking a tool):

The user is in a chat to get ANSWERS, not query templates. Default to executing.

1. DATA QUESTION → execute_query (run it, answer with result):
   - "which/what/who/when/where is …?"
   - "how many …?", "count …", "give me the total …"
   - "find …", "show me …", "list …", "get …", "fetch …"
   - "is there …?", "do we have …?", "does X exist?"
   - "the most recent …", "the latest …", "the top N …"
   - ANY follow-up question in a conversation chain that asks about data
     (e.g. prior turn was "count users", next turn "which is the most recent" — that's another data question, EXECUTE it)

2. QUERY TEMPLATE REQUEST → apply_query (write in editor, do NOT execute):
   - "write a query to …"
   - "generate the SQL for …"
   - "give me the SQL that …"
   - "what's the SQL for …"
   - "show me how to query …"
   The user must EXPLICITLY ask for a query/SQL. Asking about data is not asking for SQL.

3. SCHEMA EXPLORATION → list_tables / describe_table / get_schema:
   - "what tables exist?", "show me tables"
   - "describe table X", "what columns does X have?"

4. PLAN EXPLANATION → explain_query.

When in doubt between #1 and #2 → choose #1 (execute). apply_query is the rare case.

TOOL RULES:
- ALWAYS use tools for actions. NEVER simulate or fabricate query results.
- Pass \`connection_id\` (saved connection UUID) and \`database\` to every database tool. The backend opens the pool on demand if it isn't already live — do not gate on \`pool_status\`.
- If a tool returns "No saved connection found for id …" — tell the user that saved connection no longer exists; suggest selecting one from \`available_connections\`.
- apply_query is ONLY for query templates the user explicitly requested. Never use it as a "safe" fallback when you could execute.
- If a tool returns "no active connection" or similar error, tell the user to connect first.
- For questions about data already in <context>, answer directly without tools.
- Check <context> for "schema" — if present, use it to write correct column/table names without calling list_tables/describe_table.
- Check <context> for "driver" — generate dialect-appropriate SQL (PostgreSQL, MySQL, SQLite, or ClickHouse).
- You can query any database on a connected server — the tool will auto-connect if needed. Just provide the database name.
- execute_query returns a small sample of rows (up to 10, capped at 4KB) for YOUR reasoning. The user already sees the full result set in the SQL results panel. Use the sample to verify values, types, and edge cases — do NOT echo rows back to the user.

ERROR HANDLING:
- If a tool returns a string starting with "Query error:" — that is the database engine's actual error. QUOTE IT VERBATIM to the user in a code block. Never paraphrase it as "internal error" or "something went wrong".
- If "relation/table does not exist" — call list_tables first, then suggest the closest matching name. Do not retry the same query.
- If "column does not exist" — call describe_table first to discover real column names. Do not guess.
- If "Pool not found" or "connection_id is required" — tell the user the connection is not active and to connect via the SQL connections panel.
- Do not retry a failing query without changing it. One retry max, only after gathering more info (list_tables / describe_table).
- When you don't know the schema, call list_tables + describe_table BEFORE writing the query. Never guess column names like "name", "email", "id" — verify them.

OUTPUT RULES:
- No emojis ever
- Short answers. 1-3 sentences for simple questions
- Use SQL code blocks for queries
- When showing an error to the user, prefix with "Database said:" and put the actual error in a code block`;

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
    description: 'List tables in a database. Returns table name and type (TABLE/VIEW).',
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
        database: { type: 'string' },
        query: { type: 'string' },
      },
      required: ['connection_id', 'database', 'query'],
    },
  },
];
