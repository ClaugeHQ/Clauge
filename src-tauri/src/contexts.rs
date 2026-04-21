use crate::storage::get_storage_dir;
use std::fs;
use std::path::PathBuf;

/// Get storage dir for context snippets
fn get_contexts_dir() -> PathBuf {
    let dir = get_storage_dir().join("contexts");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// List all saved context snippets
#[tauri::command]
pub fn get_context_snippets() -> Result<Vec<serde_json::Value>, String> {
    let dir = get_contexts_dir();
    let mut snippets = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                let content = fs::read_to_string(&path).unwrap_or_default();
                let preview = content.lines().next().unwrap_or("").chars().take(80).collect::<String>();
                snippets.push(serde_json::json!({
                    "name": name,
                    "content": content,
                    "preview": preview,
                }));
            }
        }
    }
    snippets.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));
    Ok(snippets)
}

/// Save a context snippet
#[tauri::command]
pub fn save_context_snippet(name: String, content: String) -> Result<(), String> {
    let path = get_contexts_dir().join(format!("{}.md", name));
    fs::write(&path, &content).map_err(|e| e.to_string())
}

/// Delete a context snippet
#[tauri::command]
pub fn delete_context_snippet(name: String) -> Result<(), String> {
    let path = get_contexts_dir().join(format!("{}.md", name));
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Inject context snippets into CLAUDE.md for a session
#[tauri::command]
pub fn inject_session_context(project_path: String, context_names: Vec<String>) -> Result<(), String> {
    let contexts_dir = get_contexts_dir();
    let mut combined = String::new();

    for name in &context_names {
        let path = contexts_dir.join(format!("{}.md", name));
        if let Ok(content) = fs::read_to_string(&path) {
            if !combined.is_empty() { combined.push_str("\n\n---\n\n"); }
            combined.push_str(&format!("## {}\n\n{}", name, content));
        }
    }

    let claude_md_path = PathBuf::from(&project_path).join("CLAUDE.md");
    let marker_start = "<!-- CLAUGE-CONTEXT-START -->";
    let marker_end = "<!-- CLAUGE-CONTEXT-END -->";

    // Read existing content (without old markers) to check for duplicates
    let existing_content = if claude_md_path.exists() {
        let raw = fs::read_to_string(&claude_md_path).map_err(|e| e.to_string())?;
        if let (Some(start), Some(_end)) = (raw.find(marker_start), raw.find(marker_end)) {
            raw[..start].trim_end().to_string()
        } else {
            raw
        }
    } else {
        String::new()
    };

    // Filter out snippets whose content already exists in the file
    let mut filtered = String::new();
    for name in &context_names {
        let path = contexts_dir.join(format!("{}.md", name));
        if let Ok(content) = fs::read_to_string(&path) {
            if !existing_content.contains(content.trim()) {
                if !filtered.is_empty() { filtered.push_str("\n\n---\n\n"); }
                filtered.push_str(&format!("## {}\n\n{}", name, content));
            }
        }
    }

    if filtered.is_empty() { return Ok(()); }

    let injected = format!("\n\n{}\n{}\n{}\n", marker_start, filtered, marker_end);

    if !existing_content.is_empty() {
        fs::write(&claude_md_path, format!("{}{}", existing_content.trim_end(), injected)).map_err(|e| e.to_string())?;
    } else {
        fs::write(&claude_md_path, filtered).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Remove injected context from CLAUDE.md
#[tauri::command]
pub fn remove_injected_context(project_path: String) -> Result<(), String> {
    let claude_md_path = PathBuf::from(&project_path).join("CLAUDE.md");
    if !claude_md_path.exists() { return Ok(()); }

    let content = fs::read_to_string(&claude_md_path).map_err(|e| e.to_string())?;
    let marker_start = "<!-- CLAUGE-CONTEXT-START -->";
    let marker_end = "<!-- CLAUGE-CONTEXT-END -->";

    if let (Some(start), Some(end)) = (content.find(marker_start), content.find(marker_end)) {
        let cleaned = format!("{}{}", &content[..start].trim_end(), &content[end + marker_end.len()..]);
        if cleaned.trim().is_empty() {
            // We created this file — delete it
            let _ = fs::remove_file(&claude_md_path);
        } else {
            fs::write(&claude_md_path, cleaned.trim_end().to_string() + "\n").map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}
