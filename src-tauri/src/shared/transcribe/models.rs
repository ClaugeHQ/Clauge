//! ggml Whisper model catalog + on-disk manager.
//!
//! Models live in `app_data_dir/models/whisper/ggml-<name>.bin` and are
//! streamed from Hugging Face through a dedicated proxy-aware HTTP client
//! with no total timeout (the shared app client's 60s budget can't fit a
//! 466 MB download). Downloads land in a `.part` file first; only a
//! size-checked, magic-validated file is renamed into place, so a
//! half-written `.bin` can never be loaded.

use std::path::{Path, PathBuf};

use futures::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use crate::shared::http::build_download_http_client;

pub struct ModelSpec {
    pub name: &'static str,
    pub size_mb: u32,
    pub multilingual: bool,
}

pub const CATALOG: &[ModelSpec] = &[
    ModelSpec { name: "tiny", size_mb: 75, multilingual: true },
    ModelSpec { name: "tiny.en", size_mb: 75, multilingual: false },
    ModelSpec { name: "base", size_mb: 142, multilingual: true },
    ModelSpec { name: "base.en", size_mb: 142, multilingual: false },
    ModelSpec { name: "small", size_mb: 466, multilingual: true },
    ModelSpec { name: "small.en", size_mb: 466, multilingual: false },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub name: &'static str,
    pub size_mb: u32,
    pub multilingual: bool,
    pub downloaded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress<'a> {
    name: &'a str,
    downloaded: u64,
    /// 0 when the server omits Content-Length.
    total: u64,
}

/// First 4 bytes of every ggml whisper model, little-endian `0x67676d6c`
/// ("ggml") — on disk: 6c 6d 67 67. Verified against a real
/// ggml-tiny.bin download from ggerganov/whisper.cpp.
const GGML_MAGIC: [u8; 4] = [0x6c, 0x6d, 0x67, 0x67];

/// Names with an in-flight download. Vec because `Mutex::new` is const
/// while `HashSet::new` is not; the set is at most 6 entries.
static DOWNLOADING: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn spec(name: &str) -> Result<&'static ModelSpec, String> {
    CATALOG
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("Unknown whisper model '{}'", name))
}

pub fn download_url(name: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin")
}

pub fn model_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data dir: {}", e))?
        .join("models")
        .join("whisper");
    Ok(dir)
}

pub fn model_path(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    Ok(model_dir(app)?.join(format!("ggml-{name}.bin")))
}

pub fn is_downloaded(app: &AppHandle, name: &str) -> bool {
    model_path(app, name).map(|p| p.is_file()).unwrap_or(false)
}

pub fn validate_magic(path: &Path) -> bool {
    let mut buf = [0u8; 4];
    match std::fs::File::open(path) {
        Ok(mut f) => std::io::Read::read_exact(&mut f, &mut buf).is_ok() && buf == GGML_MAGIC,
        Err(_) => false,
    }
}

// ── Silero VAD model (for whisper.cpp built-in VAD) ──────────────────
// ~885 KB ggml model. Gating non-speech kills most hallucinations and
// speeds decoding (silence isn't transcribed). One file, shared across
// whisper models. Lives next to the whisper models.
const VAD_MODEL_FILE: &str = "ggml-silero-v5.1.2.bin";
const VAD_DOWNLOAD_URL: &str =
    "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin";

pub fn vad_model_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(model_dir(app)?.join(VAD_MODEL_FILE))
}

pub fn vad_model_exists(app: &AppHandle) -> bool {
    vad_model_path(app)
        .map(|p| p.is_file() && validate_magic(&p))
        .unwrap_or(false)
}

/// Ensure the Silero VAD model is present, downloading it once. Best-effort:
/// callers fall back to no-VAD on `Err` (offline first run, etc.).
pub async fn ensure_vad_model(app: &AppHandle) -> Result<PathBuf, String> {
    let final_path = vad_model_path(app)?;
    if final_path.is_file() && validate_magic(&final_path) {
        return Ok(final_path);
    }
    std::fs::create_dir_all(model_dir(app)?)
        .map_err(|e| format!("Failed to create model dir: {}", e))?;
    let pool = app.state::<SqlitePool>();
    let client = build_download_http_client(pool.inner()).await?;
    let bytes = client
        .get(VAD_DOWNLOAD_URL)
        .send()
        .await
        .map_err(|e| format!("VAD download request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("VAD download failed: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("VAD download stream error: {}", e))?;
    let part_path = final_path.with_extension("bin.part");
    std::fs::write(&part_path, &bytes)
        .map_err(|e| format!("Failed to write VAD model: {}", e))?;
    if !validate_magic(&part_path) {
        let _ = std::fs::remove_file(&part_path);
        return Err("Downloaded VAD model is not a valid ggml file".into());
    }
    std::fs::rename(&part_path, &final_path)
        .map_err(|e| format!("Failed to finalize VAD model: {}", e))?;
    Ok(final_path)
}

pub fn list_models(app: &AppHandle) -> Vec<ModelInfo> {
    CATALOG
        .iter()
        .map(|s| ModelInfo {
            name: s.name,
            size_mb: s.size_mb,
            multilingual: s.multilingual,
            downloaded: is_downloaded(app, s.name),
        })
        .collect()
}

pub fn delete_model(app: &AppHandle, name: &str) -> Result<(), String> {
    spec(name)?;
    let path = model_path(app, name)?;
    if !path.is_file() {
        return Ok(());
    }
    std::fs::remove_file(&path).map_err(|e| format!("Failed to delete model: {}", e))
}

/// Removes the name from `DOWNLOADING` on every exit path, including
/// errors bubbled with `?` and task cancellation. Once a `.part` path is
/// attached, the drop also best-effort removes it, so a cancelled download
/// (window closed, runtime shutdown) doesn't leave a stale partial file.
/// On the success path the `.part` has already been renamed away, so the
/// removal is a harmless no-op.
struct DownloadGuard {
    name: &'static str,
    part_path: Option<PathBuf>,
}

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        DOWNLOADING.lock().retain(|n| *n != self.name);
        if let Some(path) = &self.part_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub async fn download_model(app: &AppHandle, name: &str) -> Result<(), String> {
    let spec = spec(name)?;

    let mut guard = {
        let mut active = DOWNLOADING.lock();
        if active.contains(&spec.name) {
            return Err(format!("Model '{}' is already downloading", name));
        }
        active.push(spec.name);
        DownloadGuard { name: spec.name, part_path: None }
    };

    let final_path = model_path(app, name)?;
    if final_path.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(model_dir(app)?)
        .map_err(|e| format!("Failed to create model dir: {}", e))?;

    let pool = app.state::<SqlitePool>();
    let client = build_download_http_client(pool.inner()).await?;

    let part_path = final_path.with_extension("bin.part");
    guard.part_path = Some(part_path.clone());
    let result = stream_to_part(app, &client, spec.name, &part_path).await;
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part_path);
        return Err(e);
    }

    if !validate_magic(&part_path) {
        let _ = std::fs::remove_file(&part_path);
        return Err(format!(
            "Downloaded file for '{}' is not a valid ggml model",
            name
        ));
    }

    std::fs::rename(&part_path, &final_path)
        .map_err(|e| format!("Failed to finalize model file: {}", e))?;
    Ok(())
}

async fn stream_to_part(
    app: &AppHandle,
    client: &reqwest::Client,
    name: &'static str,
    part_path: &Path,
) -> Result<(), String> {
    let response = client
        .get(download_url(name))
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {}", e))?;

    let total = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(part_path)
        .await
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    const EMIT_EVERY: u64 = 2 * 1024 * 1024;
    let mut downloaded: u64 = 0;
    let mut last_emitted: u64 = 0;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("Failed to write model file: {}", e))?;
        downloaded += chunk.len() as u64;
        if downloaded - last_emitted >= EMIT_EVERY || downloaded == total {
            last_emitted = downloaded;
            let _ = app.emit(
                "meetings:model-download-progress",
                DownloadProgress { name, downloaded, total },
            );
        }
    }

    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| format!("Failed to flush model file: {}", e))?;

    // A dropped connection can end the stream cleanly without delivering
    // every byte; catch it here so a truncated file never reaches the
    // (header-only) magic validation.
    if total > 0 && downloaded != total {
        return Err(format!(
            "Download of '{}' incomplete ({}/{} bytes)",
            name, downloaded, total
        ));
    }

    let _ = app.emit(
        "meetings:model-download-progress",
        DownloadProgress { name, downloaded, total },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_url_maps_name_into_hf_path() {
        assert_eq!(
            download_url("tiny"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"
        );
        assert_eq!(
            download_url("base.en"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
        );
    }

    #[test]
    fn catalog_has_six_models() {
        assert_eq!(CATALOG.len(), 6);
        assert!(CATALOG.iter().any(|s| s.name == "tiny"));
        assert!(CATALOG.iter().any(|s| s.name == "small.en"));
        for spec in CATALOG {
            assert_eq!(spec.multilingual, !spec.name.ends_with(".en"));
        }
    }

    #[test]
    fn validate_magic_accepts_ggml_header() {
        let path = std::env::temp_dir().join("clauge-test-ggml-magic-ok.bin");
        std::fs::write(&path, [0x6c, 0x6d, 0x67, 0x67, 0x01, 0x02, 0x03]).unwrap();
        assert!(validate_magic(&path));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn validate_magic_rejects_other_content() {
        let path = std::env::temp_dir().join("clauge-test-ggml-magic-bad.bin");
        std::fs::write(&path, b"<!DOCTYPE html><html>error page</html>").unwrap();
        assert!(!validate_magic(&path));
        let _ = std::fs::remove_file(&path);

        let short = std::env::temp_dir().join("clauge-test-ggml-magic-short.bin");
        std::fs::write(&short, [0x6c]).unwrap();
        assert!(!validate_magic(&short));
        let _ = std::fs::remove_file(&short);

        assert!(!validate_magic(Path::new("/nonexistent/clauge/ggml.bin")));
    }
}
