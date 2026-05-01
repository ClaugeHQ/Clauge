export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // CORS headers
    const corsHeaders = {
      'Access-Control-Allow-Origin': 'https://clauge.in',
      'Access-Control-Allow-Methods': 'POST, OPTIONS',
      'Access-Control-Allow-Headers': 'Content-Type',
    };

    // Handle preflight
    if (request.method === 'OPTIONS') {
      return new Response(null, { headers: corsHeaders });
    }

    // Only handle POST /auth/token
    if (url.pathname === '/auth/token' && request.method === 'POST') {
      try {
        const { code } = await request.json();
        if (!code) {
          return Response.json({ error: 'Missing code' }, { status: 400, headers: corsHeaders });
        }

        // Exchange code for access token
        const tokenResp = await fetch('https://github.com/login/oauth/access_token', {
          method: 'POST',
          headers: {
            'Accept': 'application/json',
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            client_id: env.GITHUB_CLIENT_ID,
            client_secret: env.GITHUB_CLIENT_SECRET,
            code: code,
          }),
        });

        const data = await tokenResp.json();

        if (data.error) {
          return Response.json({ error: data.error_description || data.error }, { status: 400, headers: corsHeaders });
        }

        return Response.json({ access_token: data.access_token }, { headers: corsHeaders });
      } catch (e) {
        return Response.json({ error: 'Internal error' }, { status: 500, headers: corsHeaders });
      }
    }

    return new Response('Not found', { status: 404 });
  },
};
