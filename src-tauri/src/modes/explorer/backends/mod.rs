//! `RemoteFs` implementations per protocol. Each lands in a separate stage
//! (SFTP → 2, S3 → 3, Azure → 4, FTP → 5).

pub mod azure_blob;
pub mod ftp;
pub mod s3;
pub mod s3_presets;
pub mod sftp;

/// Best-effort mime detection by extension. Used by every backend's
/// `stat()` so the frontend can pick a viewer (text / image / hex).
pub fn mime_for(path: &str) -> Option<String> {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_lowercase())?;
    let mime = match ext.as_str() {
        "txt" | "log" | "md" | "rs" | "ts" | "js" | "py" | "go" | "rb" | "java" | "c" | "cpp"
        | "h" | "yaml" | "yml" | "toml" | "json" | "xml" | "html" | "css" | "sh" | "ini"
        | "conf" | "csv" | "tsv" | "sql" => "text/plain",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        _ => return None,
    };
    Some(mime.to_string())
}

/// Translate a POSIX-style glob (`*`, `?`) to a regex anchored to a single
/// path segment. Used by all backends' recursive `search()`.
pub fn glob_pattern(glob: &str) -> Result<regex::Regex, String> {
    let mut re = String::from("^");
    for ch in glob.chars() {
        match ch {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' => re.push(c),
            c => {
                re.push('\\');
                re.push(c);
            }
        }
    }
    re.push('$');
    regex::Regex::new(&re).map_err(|e| e.to_string())
}

/// Join a parent path with a child name, preserving leading slash.
pub fn posix_join(parent: &str, child: &str) -> String {
    if parent.is_empty() || parent == "/" {
        format!("/{}", child.trim_start_matches('/'))
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), child.trim_start_matches('/'))
    }
}
