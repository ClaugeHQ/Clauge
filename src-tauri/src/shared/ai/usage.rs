use sqlx::SqlitePool;
use tauri::State;

use super::types::{AiProviderStat, AiUsageStat};
use crate::shared::repos::ai_usage as ai_usage_repo;

#[tauri::command]
pub async fn get_ai_usage_stats(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<AiUsageStat>, String> {
    let stats = ai_usage_repo::stats_by_mode(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    Ok(stats
        .into_iter()
        .map(|(mode, total_calls, input_tokens, output_tokens, total_tool_rounds, max_tool_rounds)| AiUsageStat {
            mode,
            total_calls,
            input_tokens,
            output_tokens,
            total_tool_rounds,
            max_tool_rounds,
        })
        .collect())
}

#[tauri::command]
pub async fn get_ai_provider_stats(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<AiProviderStat>, String> {
    let stats = ai_usage_repo::stats_by_model(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    Ok(stats
        .into_iter()
        .map(|(model, total_calls, input_tokens, output_tokens, total_tool_rounds, max_tool_rounds)| AiProviderStat {
            model,
            total_calls,
            input_tokens,
            output_tokens,
            total_tool_rounds,
            max_tool_rounds,
        })
        .collect())
}

#[tauri::command]
pub async fn reset_ai_usage(pool: State<'_, SqlitePool>) -> Result<(), String> {
    ai_usage_repo::clear_all(pool.inner())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn record_ai_usage(
    pool: State<'_, SqlitePool>,
    mode: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    tool_rounds: Option<i64>,
) -> Result<(), String> {
    let id = uuid::Uuid::new_v4().to_string();
    ai_usage_repo::record(
        pool.inner(),
        &id,
        &mode,
        &model,
        input_tokens,
        output_tokens,
        tool_rounds.unwrap_or(0),
    )
    .await
    .map_err(|e| e.to_string())
}
