# Worker setup notes

## Push relay (`POST /api/push/send`)

Requires a Firebase service-account JSON (fields `client_email`, `private_key`, `project_id`) stored as a secret:

```sh
wrangler secret put FCM_SERVICE_ACCOUNT   # paste the full service-account JSON
```

Without the secret the endpoint returns `503 {"error":"push_not_configured"}`.
