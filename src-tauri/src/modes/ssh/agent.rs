//! SSH-agent authentication.
//!
//! Connects to the running ssh-agent (Unix domain socket on Unix,
//! Windows named pipe on Windows), lists available identities, and
//! attempts to authenticate against the SSH server using each identity
//! in turn.
//!
//! This is the only auth path that supports hardware-backed keys
//! (YubiKey PIV, smartcards) — those keys never leave the device, so
//! file-based loading is impossible. The agent signs auth challenges
//! using the device on our behalf.

use russh::client::Handle;
use russh_keys::agent::client::AgentClient;
use tokio::io::{AsyncRead, AsyncWrite};

/// Try authenticating to `handle` as `username` using whatever keys the
/// running ssh-agent holds. Returns `Ok(true)` on success, `Ok(false)`
/// if no agent key was accepted by the server. Returns `Err` on agent
/// connection / protocol failures.
pub async fn try_agent_auth<H>(
    handle: &mut Handle<H>,
    username: &str,
) -> Result<bool, String>
where
    H: russh::client::Handler,
{
    #[cfg(unix)]
    {
        let path = std::env::var("SSH_AUTH_SOCK")
            .map_err(|_| "SSH_AUTH_SOCK is not set — start ssh-agent or load a key".to_string())?;
        let agent = AgentClient::connect_uds(&path)
            .await
            .map_err(|e| format!("connect ssh-agent at {}: {}", path, e))?;
        try_each_identity(handle, username, agent).await
    }

    #[cfg(windows)]
    {
        // OpenSSH for Windows ships an ssh-agent service that exposes its
        // socket as a named pipe. The pipe name is documented in OpenSSH-Win
        // and stable across Windows 10/11.
        const PIPE_NAME: &str = r"\\.\pipe\openssh-ssh-agent";
        let pipe = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(PIPE_NAME)
            .map_err(|e| {
                format!(
                    "connect ssh-agent at {}: {}. Is the OpenSSH Authentication Agent service running?",
                    PIPE_NAME, e
                )
            })?;
        let agent = AgentClient::connect(pipe);
        try_each_identity(handle, username, agent).await
    }
}

/// Inner loop — generic over the agent's stream type so it works for both
/// `AgentClient<UnixStream>` and `AgentClient<NamedPipeClient>`.
///
/// We try each identity sequentially. The agent gives us PublicKey objects
/// (no private material), and `authenticate_future` delegates the actual
/// signing back to the agent. After each attempt the handle returns the
/// agent so we can reuse it for the next key.
async fn try_each_identity<S, H>(
    handle: &mut Handle<H>,
    username: &str,
    mut agent: AgentClient<S>,
) -> Result<bool, String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    H: russh::client::Handler,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| format!("request agent identities: {}", e))?;

    if identities.is_empty() {
        return Err("ssh-agent has no keys loaded — run `ssh-add` first".to_string());
    }

    for pubkey in identities {
        let (returned_agent, result) = handle
            .authenticate_future(username, pubkey, agent)
            .await;
        agent = returned_agent;
        match result {
            Ok(true) => return Ok(true),
            // The server didn't accept this key — try the next one. Most
            // OpenSSH configs only authorize one or two of a user's keys.
            Ok(false) => continue,
            Err(e) => return Err(format!("agent auth attempt: {}", e)),
        }
    }
    Ok(false)
}
