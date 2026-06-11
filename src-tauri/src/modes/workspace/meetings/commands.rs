use sqlx::SqlitePool;
use tauri::{AppHandle, State};

use crate::modes::workspace::meetings::detect;
use crate::modes::workspace::meetings::recorder;
use crate::modes::workspace::meetings::repo;
use crate::modes::workspace::models::WorkspaceMeeting;
use crate::shared::repos::settings as settings_repo;
use crate::shared::transcribe::models as whisper_models;

// --- CRUD ---

/// Transcripts can run to megabytes; the list view never renders them,
/// so each row's `transcript` is blanked to "[]" — `workspace_meeting_get`
/// is the only way to load the full transcript.
#[tauri::command]
pub async fn workspace_meeting_list(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<WorkspaceMeeting>, String> {
    let mut meetings = repo::list_meetings(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
    for meeting in &mut meetings {
        meeting.transcript = "[]".to_string();
    }
    Ok(meetings)
}

#[tauri::command]
pub async fn workspace_meeting_get(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<WorkspaceMeeting, String> {
    repo::get_meeting(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Meeting not found".to_string())
}

#[tauri::command]
pub async fn workspace_meeting_update_title(
    pool: State<'_, SqlitePool>,
    id: String,
    title: String,
) -> Result<(), String> {
    let rows = repo::update_title(pool.inner(), &id, &title)
        .await
        .map_err(|e| e.to_string())?;
    if rows == 0 {
        return Err("Meeting not found".to_string());
    }
    Ok(())
}

/// Manual edit path — provider/model stay untouched (None), so the
/// "AI generated" stamp is never applied to hand-written notes.
#[tauri::command]
pub async fn workspace_meeting_update_notes(
    pool: State<'_, SqlitePool>,
    id: String,
    notes_md: String,
) -> Result<(), String> {
    let rows = repo::update_notes(pool.inner(), &id, &notes_md, None, None)
        .await
        .map_err(|e| e.to_string())?;
    if rows == 0 {
        return Err("Meeting not found".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn workspace_meeting_delete(
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    repo::delete_meeting(pool.inner(), &id)
        .await
        .map_err(|e| e.to_string())
}

// --- Whisper models ---

#[tauri::command]
pub async fn workspace_meeting_models_list(
    app: AppHandle,
) -> Result<Vec<whisper_models::ModelInfo>, String> {
    Ok(whisper_models::list_models(&app))
}

#[tauri::command]
pub async fn workspace_meeting_model_download(
    app: AppHandle,
    name: String,
) -> Result<(), String> {
    whisper_models::download_model(&app, &name).await
}

#[tauri::command]
pub async fn workspace_meeting_model_delete(
    app: AppHandle,
    name: String,
) -> Result<(), String> {
    whisper_models::delete_model(&app, &name)
}

// --- Recording ---

/// Settings keys for the user's default transcription model/language.
/// Written by the Settings UI through the generic `set_setting` command;
/// read here so callers that can't reach the frontend settings store
/// (the floating widget) inherit the same defaults.
pub const MODEL_SETTING_KEY: &str = "workspace_meeting_model";
pub const LANGUAGE_SETTING_KEY: &str = "workspace_meeting_language";

async fn setting_or(pool: &SqlitePool, key: &str, default: &str) -> String {
    match settings_repo::get_by_key(pool, key).await {
        Ok(Some(s)) if !s.value.trim().is_empty() => s.value,
        _ => default.to_string(),
    }
}

#[tauri::command]
pub async fn workspace_meeting_start(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    source_app: Option<String>,
    model: Option<String>,
    language: Option<String>,
) -> Result<String, String> {
    let model = match model {
        Some(m) => m,
        None => setting_or(pool.inner(), MODEL_SETTING_KEY, recorder::DEFAULT_MODEL).await,
    };
    let language = match language {
        Some(l) => l,
        None => {
            setting_or(
                pool.inner(),
                LANGUAGE_SETTING_KEY,
                recorder::DEFAULT_LANGUAGE,
            )
            .await
        }
    };
    recorder::start_recording(app, source_app, model, language).await
}

#[tauri::command]
pub async fn workspace_meeting_stop(app: AppHandle) -> Result<String, String> {
    recorder::stop_recording(app).await
}

#[tauri::command]
pub fn workspace_meeting_recording_status(
    state: State<'_, recorder::RecorderState>,
) -> recorder::RecorderStatus {
    state.status()
}

// --- Call detection ---

#[tauri::command]
pub async fn workspace_meeting_detect_set_enabled(
    pool: State<'_, SqlitePool>,
    detect_state: State<'_, detect::DetectState>,
    enabled: bool,
) -> Result<(), String> {
    settings_repo::upsert(
        pool.inner(),
        detect::SETTING_KEY,
        if enabled { "true" } else { "false" },
    )
    .await
    .map_err(|e| e.to_string())?;
    detect_state.set_enabled(enabled);
    Ok(())
}

#[tauri::command]
pub fn workspace_meeting_detect_get_enabled(
    detect_state: State<'_, detect::DetectState>,
) -> bool {
    detect_state.enabled()
}

#[tauri::command]
pub fn workspace_meeting_detect_dismiss(detect_state: State<'_, detect::DetectState>) {
    detect_state.dismiss();
}

/// Snapshot for widget re-sync after a webview reload: did it miss a
/// `meetings:call-detected` while it wasn't listening?
#[tauri::command]
pub fn workspace_meeting_detect_status(
    detect_state: State<'_, detect::DetectState>,
) -> detect::DetectStatus {
    detect_state.status()
}
