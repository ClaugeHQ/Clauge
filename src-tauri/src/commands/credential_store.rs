use async_trait::async_trait;

/// Cross-platform secret store used to back SSH profile credentials
/// (passwords + key passphrases). Phase 1 ships a macOS Keychain impl;
/// Windows / Linux backends will plug in here later.
#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn store(&self, key: &str, value: &str) -> Result<(), String>;
    async fn get(&self, key: &str) -> Result<Option<String>, String>;
    async fn delete(&self, key: &str) -> Result<(), String>;
}

const SERVICE_NAME: &str = "Clauge SSH";

#[cfg(target_os = "macos")]
pub struct MacosKeychainStore;

#[cfg(target_os = "macos")]
#[async_trait]
impl CredentialStore for MacosKeychainStore {
    async fn store(&self, key: &str, value: &str) -> Result<(), String> {
        let key = key.to_string();
        let value = value.to_string();
        tokio::task::spawn_blocking(move || {
            let output = std::process::Command::new("security")
                .args([
                    "add-generic-password",
                    "-U", // update if exists
                    "-s",
                    SERVICE_NAME,
                    "-a",
                    &key,
                    "-w",
                    &value,
                ])
                .output()
                .map_err(|e| format!("security add-generic-password spawn failed: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!("security add-generic-password failed: {}", stderr));
            }
            Ok(())
        })
        .await
        .map_err(|e| format!("join error: {}", e))?
    }

    async fn get(&self, key: &str) -> Result<Option<String>, String> {
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let output = std::process::Command::new("security")
                .args([
                    "find-generic-password",
                    "-s",
                    SERVICE_NAME,
                    "-a",
                    &key,
                    "-w",
                ])
                .output()
                .map_err(|e| format!("security find-generic-password spawn failed: {}", e))?;
            if !output.status.success() {
                // Not found → Ok(None). `security` returns non-zero with empty stdout.
                let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
                if stderr.contains("could not be found") || stderr.is_empty() {
                    return Ok(None);
                }
                return Err(format!(
                    "security find-generic-password failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            let mut s = String::from_utf8_lossy(&output.stdout).to_string();
            // `security -w` prints the password followed by a newline.
            if s.ends_with('\n') {
                s.pop();
            }
            if s.ends_with('\r') {
                s.pop();
            }
            Ok(Some(s))
        })
        .await
        .map_err(|e| format!("join error: {}", e))?
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            // Best-effort: ignore "not found" errors so this is idempotent.
            let _ = std::process::Command::new("security")
                .args([
                    "delete-generic-password",
                    "-s",
                    SERVICE_NAME,
                    "-a",
                    &key,
                ])
                .output();
            Ok(())
        })
        .await
        .map_err(|e| format!("join error: {}", e))?
    }
}

#[cfg(not(target_os = "macos"))]
pub struct StubCredentialStore;

#[cfg(not(target_os = "macos"))]
#[async_trait]
impl CredentialStore for StubCredentialStore {
    async fn store(&self, _key: &str, _value: &str) -> Result<(), String> {
        Err("Credential store not yet implemented for this platform (macOS only in phase 1)"
            .to_string())
    }
    async fn get(&self, _key: &str) -> Result<Option<String>, String> {
        Err("Credential store not yet implemented for this platform (macOS only in phase 1)"
            .to_string())
    }
    async fn delete(&self, _key: &str) -> Result<(), String> {
        // Make delete a no-op on unsupported platforms so profile cleanup stays unblocked.
        Ok(())
    }
}

/// Returns the platform-appropriate credential store implementation.
#[cfg(target_os = "macos")]
pub fn credential_store() -> impl CredentialStore {
    MacosKeychainStore
}

#[cfg(not(target_os = "macos"))]
pub fn credential_store() -> impl CredentialStore {
    StubCredentialStore
}
