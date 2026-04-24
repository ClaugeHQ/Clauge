export const REST_SYSTEM_PROMPT = `You are a REST API assistant inside Clauge app. You help users with HTTP requests.

CONTEXT: The user's current request and response are in <context> tags. Read them before answering.

TOOL RULES:
- ALWAYS use tools for actions. NEVER simulate, guess, or fabricate results.
- CRITICAL: When the user mentions a request by NAME (e.g. "test Get User by ID", "run Login API"):
  1. First call list_collections to find the request_id
  2. Then call execute_request with that request_id
  DO NOT use execute_current_request — that only works for the tab currently open.
- "run/execute/send this" (no name given, referring to active tab) → use execute_current_request
- "run/execute collection X" → use execute_collection (accepts collection name or ID)
- "create/build/generate a POST to /users" → use apply_request. ONLY for creating NEW requests.
- apply_request must NEVER be used when the user asks to execute/run/send.
- If a tool fails, try a different approach (e.g. list_collections to find the correct ID). Do NOT give up and ask the user for an ID.
- NEVER make up HTTP responses, status codes, or error messages. Always use the tools to get real data.

OUTPUT RULES:
- No emojis ever
- Short answers. 1-3 sentences max for simple questions
- Use code blocks only for JSON or code
- When a tool returns "displayed to user" or "report card", say only "Done." or a brief summary
- Do not repeat data the user can already see
- Do not create tables or lists for execution results`;

export const REST_TOOLS = [
  {
    name: 'list_collections',
    description: 'List all collections with their requests (name, method, URL).',
    input_schema: { type: 'object' as const, properties: {}, required: [] as string[] },
  },
  {
    name: 'get_request_details',
    description: 'Get full details of a saved request by ID.',
    input_schema: {
      type: 'object' as const,
      properties: {
        request_id: { type: 'string' },
      },
      required: ['request_id'],
    },
  },
  {
    name: 'list_environments',
    description: 'List environments and their variables (secrets masked).',
    input_schema: { type: 'object' as const, properties: {}, required: [] as string[] },
  },
  {
    name: 'get_history',
    description: 'Get recent request history.',
    input_schema: {
      type: 'object' as const,
      properties: {
        limit: { type: 'integer' },
      },
      required: [] as string[],
    },
  },
  {
    name: 'execute_current_request',
    description: 'Run the request in the currently open tab. ONLY use when user says "run this", "send this" without naming a specific request. Do NOT use this when the user mentions a request by name — use execute_request instead.',
    input_schema: { type: 'object' as const, properties: {}, required: [] as string[] },
  },
  {
    name: 'apply_request',
    description: 'Create or modify an HTTP request and show it to the user. Only use when user asks to CREATE, BUILD, or GENERATE a request. Never use for execute/run/send.',
    input_schema: {
      type: 'object' as const,
      properties: {
        method: { type: 'string', enum: ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'] },
        url: { type: 'string' },
        headers: {
          type: 'array',
          items: {
            type: 'object',
            properties: { key: { type: 'string' }, value: { type: 'string' } },
            required: ['key', 'value'],
          },
        },
        body: { type: 'string' },
        body_type: { type: 'string', enum: ['json', 'text', 'xml', 'form', 'none'] },
        params: {
          type: 'array',
          items: {
            type: 'object',
            properties: { key: { type: 'string' }, value: { type: 'string' } },
            required: ['key', 'value'],
          },
        },
      },
      required: ['method', 'url'],
    },
  },
  {
    name: 'execute_request',
    description: 'Execute a saved request by name or ID and return the response. Use when the user mentions a specific request name like "test Get User by ID". The request_id can be a UUID or the request name.',
    input_schema: {
      type: 'object' as const,
      properties: {
        request_id: { type: 'string' },
        environment_id: { type: 'string' },
      },
      required: ['request_id'],
    },
  },
  {
    name: 'create_request',
    description: 'Create a new request in a collection.',
    input_schema: {
      type: 'object' as const,
      properties: {
        collection_id: { type: 'string' },
        name: { type: 'string' },
        method: { type: 'string', enum: ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'] },
        url: { type: 'string' },
      },
      required: ['collection_id', 'name', 'method', 'url'],
    },
  },
  {
    name: 'execute_collection',
    description: 'Execute all requests in a collection sequentially.',
    input_schema: {
      type: 'object' as const,
      properties: {
        collection_id: { type: 'string' },
        environment_id: { type: 'string' },
      },
      required: ['collection_id'],
    },
  },
  {
    name: 'generate_curl',
    description: 'Generate a cURL command from a saved request by ID.',
    input_schema: {
      type: 'object' as const,
      properties: {
        request_id: { type: 'string' },
      },
      required: ['request_id'],
    },
  },
  {
    name: 'switch_environment',
    description: 'Switch the active environment. Use when user asks to change to a different environment.',
    input_schema: {
      type: 'object' as const,
      properties: {
        environment_id: { type: 'string' },
      },
      required: ['environment_id'],
    },
  },
  {
    name: 'rename_request',
    description: 'Rename a saved request.',
    input_schema: {
      type: 'object' as const,
      properties: {
        request_id: { type: 'string' },
        new_name: { type: 'string' },
      },
      required: ['request_id', 'new_name'],
    },
  },
  {
    name: 'delete_request',
    description: 'Delete a saved request from a collection.',
    input_schema: {
      type: 'object' as const,
      properties: {
        request_id: { type: 'string' },
      },
      required: ['request_id'],
    },
  },
  {
    name: 'duplicate_request',
    description: 'Duplicate an existing request in the same collection.',
    input_schema: {
      type: 'object' as const,
      properties: {
        request_id: { type: 'string' },
      },
      required: ['request_id'],
    },
  },
  {
    name: 'create_collection',
    description: 'Create a new collection to organize requests.',
    input_schema: {
      type: 'object' as const,
      properties: {
        name: { type: 'string' },
        description: { type: 'string' },
      },
      required: ['name'],
    },
  },
  {
    name: 'set_env_variable',
    description: 'Create or update an environment variable. Useful for capturing tokens from responses.',
    input_schema: {
      type: 'object' as const,
      properties: {
        environment_id: { type: 'string' },
        key: { type: 'string' },
        value: { type: 'string' },
        is_secret: { type: 'boolean' },
      },
      required: ['environment_id', 'key', 'value'],
    },
  },
  {
    name: 'search_history',
    description: 'Search request history by URL pattern or status code. Supports status codes like "404" or ranges like "4xx", "5xx".',
    input_schema: {
      type: 'object' as const,
      properties: {
        query: { type: 'string', description: 'URL pattern to search for' },
        status: { type: 'string', description: 'Status code (e.g. "404") or range (e.g. "4xx")' },
        limit: { type: 'integer', description: 'Max results (default 20, max 50)' },
      },
      required: [] as string[],
    },
  },
];
