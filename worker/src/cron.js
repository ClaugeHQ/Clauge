// Daily/hourly sweep: revoke users whose subscription has been past_due
// for more than 3 days. Per spec §8.2, we don't rely on a webhook firing
// at exactly day-3 — Polar's retries can land at any point.

export async function sweepPastDue(env) {
  const result = await env.CLAUGE_DB.prepare(
    `UPDATE users SET
       plan = 'free',
       subscription_status = 'canceled',
       cancel_at_period_end = 0,
       credits_remaining = 0,
       past_due_started_at = NULL,
       updated_at = CURRENT_TIMESTAMP
     WHERE subscription_status = 'past_due'
       AND past_due_started_at < datetime('now', '-3 days')`
  ).run();
  // D1 returns meta with changes count.
  return result.meta?.changes ?? 0;
}
