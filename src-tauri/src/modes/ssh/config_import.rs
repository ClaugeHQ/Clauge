//! Read and import host entries from `~/.ssh/config`.
//!
//! OpenSSH config syntax is identical across macOS/Linux/Windows (Win10+
//! ships an OpenSSH client by default), so this is a single cross-platform
//! feature with no per-OS branching. The parser handles the subset of
//! directives we model on `SshProfile`:
//!   - `Host alias`           → profile name
//!   - `HostName x`           → host
//!   - `User x`               → username
//!   - `Port n`               → port (default 22)
//!   - `IdentityFile path`    → key_path (auth_type = "key")
//!
//! `Match`, `Include`, and pattern/wildcard `Host *` blocks are skipped —
//! they don't represent a single connectable host. Comments (`#`) and
//! blank lines are ignored. `=` and whitespace both work as the
//! key/value separator (per ssh_config(5)).

use crate::shared::repos::ssh_profiles as ssh_profiles_repo;
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashSet;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigHost {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: i64,
    pub identity_file: Option<String>,
    /// True if a profile with `name == alias` is already in the DB.
    /// The UI shows these greyed out and excludes them from import.
    pub already_exists: bool,
}

#[tauri::command]
pub async fn ssh_read_config_hosts(
    pool: State<'_, SqlitePool>,
) -> Result<Vec<SshConfigHost>, String> {
    let path = match dirs::home_dir() {
        Some(home) => home.join(".ssh").join("config"),
        None => return Ok(Vec::new()),
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;
    let mut hosts = parse(&content);

    let existing_names: HashSet<String> = ssh_profiles_repo::list_all(pool.inner())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| p.name)
        .collect();
    for h in &mut hosts {
        h.already_exists = existing_names.contains(&h.alias);
    }
    Ok(hosts)
}

#[tauri::command]
pub async fn ssh_import_config_hosts(
    pool: State<'_, SqlitePool>,
    aliases: Vec<String>,
) -> Result<usize, String> {
    let path = match dirs::home_dir() {
        Some(home) => home.join(".ssh").join("config"),
        None => return Err("home directory not available".into()),
    };
    if !path.exists() {
        return Err("~/.ssh/config does not exist".into());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let hosts = parse(&content);

    let existing_names: HashSet<String> = ssh_profiles_repo::list_all(pool.inner())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| p.name)
        .collect();

    let wanted: HashSet<String> = aliases.into_iter().collect();
    let mut imported = 0usize;

    for h in hosts {
        if !wanted.contains(&h.alias) || existing_names.contains(&h.alias) {
            continue;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let auth_type = if h.identity_file.is_some() { "key" } else { "password" };
        let username = h.user.as_deref().unwrap_or("");
        ssh_profiles_repo::insert(
            pool.inner(),
            &id,
            &h.alias,
            &h.hostname,
            h.port,
            username,
            auth_type,
            h.identity_file.as_deref(),
            None,
            &now,
        )
        .await
        .map_err(|e| e.to_string())?;
        imported += 1;
    }
    Ok(imported)
}

// ── Parser ──────────────────────────────────────────────────────────────────

struct HostBuilder {
    alias: String,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<i64>,
    identity_file: Option<String>,
}

impl HostBuilder {
    fn new(alias: String) -> Self {
        Self {
            alias,
            hostname: None,
            user: None,
            port: None,
            identity_file: None,
        }
    }

    fn build(self) -> SshConfigHost {
        SshConfigHost {
            hostname: self.hostname.clone().unwrap_or_else(|| self.alias.clone()),
            alias: self.alias,
            user: self.user,
            port: self.port.unwrap_or(22),
            identity_file: self.identity_file,
            already_exists: false,
        }
    }
}

fn parse(content: &str) -> Vec<SshConfigHost> {
    let mut out = Vec::new();
    let mut current: Option<HostBuilder> = None;

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let split_idx = line
            .char_indices()
            .find(|&(_, c)| c.is_whitespace() || c == '=')
            .map(|(i, _)| i);
        let (key, value) = match split_idx {
            Some(i) => {
                let v = line[i..]
                    .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
                    .trim();
                (line[..i].to_lowercase(), v)
            }
            None => continue,
        };

        match key.as_str() {
            "host" => {
                if let Some(h) = current.take() {
                    out.push(h.build());
                }
                // Skip wildcards / patterns / multi-aliases — we model 1:1 hosts.
                if value.is_empty()
                    || value.contains('*')
                    || value.contains('?')
                    || value.contains(' ')
                {
                    current = None;
                } else {
                    current = Some(HostBuilder::new(value.to_string()));
                }
            }
            "match" => {
                if let Some(h) = current.take() {
                    out.push(h.build());
                }
                current = None;
            }
            "hostname" => {
                if let Some(c) = current.as_mut() {
                    c.hostname = Some(value.to_string());
                }
            }
            "user" => {
                if let Some(c) = current.as_mut() {
                    c.user = Some(value.to_string());
                }
            }
            "port" => {
                if let Some(c) = current.as_mut() {
                    if let Ok(p) = value.parse::<i64>() {
                        c.port = Some(p);
                    }
                }
            }
            "identityfile" => {
                if let Some(c) = current.as_mut() {
                    // OpenSSH allows multiple IdentityFile lines; we keep the
                    // first since SshProfile carries a single key path.
                    if c.identity_file.is_none() {
                        c.identity_file = Some(resolve_identity_path(unquote(value)));
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(h) = current.take() {
        out.push(h.build());
    }
    out
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"'))
            || (s.starts_with('\'') && s.ends_with('\'')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Resolve an `IdentityFile` value to an absolute on-disk path, tolerating
/// the variety of forms that show up in real `~/.ssh/config` files on macOS,
/// Linux, and Windows.
///
/// Handles, in priority order:
///   1. `~`                         → user's home dir
///   2. `%d/...`                    → OpenSSH macro for home dir, expanded in place
///   3. `~/<home-dir-components>/…` → user wrote `~/Users/<me>/...` (macOS),
///                                    `~/home/<me>/...` (Linux), or
///                                    `~/Users/<me>/...` / `~/C:/Users/<me>/...`
///                                    (Windows) — i.e. a tilde stuck in front
///                                    of a path that's already absolute.
///                                    Detected via component-wise comparison so
///                                    it works across all three OSes regardless
///                                    of `/` vs `\` separators or drive prefixes.
///   4. `~/path/dne` BUT `/path/dne` exists (Unix) → treat as already-absolute.
///   5. `~/path` (standard)         → `$HOME/path`
///   6. Relative path               → `$HOME/.ssh/<path>` per OpenSSH default
///   7. Absolute path               → returned unchanged
///
/// `%h`, `%r`, `%u` are deliberately NOT expanded — they require connect-time
/// values we don't have during import. Paths containing them stay literal and
/// fail at connect time with a clear error in the UI.
fn resolve_identity_path(input: &str) -> String {
    use std::path::{Path, PathBuf};

    let input = input.trim();
    let Some(home) = dirs::home_dir() else {
        return input.to_string();
    };

    // %d → home dir. (Only this OpenSSH macro is import-time expandable.)
    let expanded = input.replacen("%d", &home.to_string_lossy(), 1);
    let path = expanded.as_str();

    // Bare `~` → home dir.
    if path == "~" {
        return home.to_string_lossy().to_string();
    }

    // `~/<rest>` cases.
    if let Some(after) = path.strip_prefix("~/") {
        if let Some(recovered) = recover_doubled_home_prefix(&home, after) {
            return recovered;
        }

        let naive = home.join(after);
        if naive.exists() {
            return naive.to_string_lossy().to_string();
        }

        // Existence fallback — Unix only. On Windows, prefixing `/` to a
        // path doesn't yield a valid absolute path (needs a drive letter),
        // so this branch wouldn't help there.
        #[cfg(unix)]
        {
            let absolute_alt = PathBuf::from(format!("/{}", after));
            if absolute_alt.exists() {
                return absolute_alt.to_string_lossy().to_string();
            }
        }

        return naive.to_string_lossy().to_string();
    }

    // No tilde. Relative paths resolve against ~/.ssh/ per OpenSSH convention.
    let p = Path::new(path);
    if p.is_relative() {
        return home.join(".ssh").join(p).to_string_lossy().to_string();
    }

    path.to_string()
}

/// Detect the "tilde + already-absolute path" user error and recover, in
/// a way that works on macOS, Linux, AND Windows.
///
/// Examples:
///   home=/Users/macbook,    after=Users/macbook/ssh/foo.pem
///     → /Users/macbook/ssh/foo.pem
///   home=/home/me,          after=home/me/.ssh/id_rsa
///     → /home/me/.ssh/id_rsa
///   home=C:\Users\me,       after=Users/me/.ssh/id_rsa
///     → C:\Users\me\.ssh\id_rsa
///   home=C:\Users\me,       after=C:/Users/me/.ssh/id_rsa
///     → C:\Users\me\.ssh\id_rsa
///
/// The trick is to compare *path components* rather than raw strings —
/// `Path::components()` handles drive prefixes (`C:`), root markers (`/`),
/// and mixed `/` vs `\` separators correctly per OS, so the same code
/// recovers all four cases above without per-OS branching.
fn recover_doubled_home_prefix(home: &std::path::Path, after: &str) -> Option<String> {
    use std::path::{Component, PathBuf};

    // The "name-bearing" components of home — strip drive prefix and root
    // marker so we're comparing just the directory names.
    let home_names: Vec<_> = home
        .components()
        .filter(|c| !matches!(c, Component::Prefix(_) | Component::RootDir | Component::CurDir))
        .collect();
    if home_names.is_empty() {
        // Defensive: if home is something like `/` or `C:\`, there's nothing
        // to detect against. Fall through to naive expansion.
        return None;
    }

    let after_path = PathBuf::from(after);
    let mut after_iter = after_path
        .components()
        .filter(|c| !matches!(c, Component::Prefix(_) | Component::RootDir | Component::CurDir));

    // Does `after` start with the same name-components as `home`?
    for hc in &home_names {
        match after_iter.next() {
            Some(ac) if ac == *hc => continue,
            _ => return None,
        }
    }

    // Yes. Reconstruct: home + remaining after-components.
    let remaining: PathBuf = after_iter.collect();
    Some(home.join(remaining).to_string_lossy().to_string())
}
