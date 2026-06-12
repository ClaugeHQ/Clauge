/// Returns the macOS version as `(major, minor)` via `sw_vers`, or `None`
/// when the lookup or parse fails. Callers gate version-dependent behavior
/// (Core Audio tap support, legacy window content protection) on this.
pub fn macos_version() -> Option<(u32, u32)> {
    let out = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let mut parts = text.trim().split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor))
}
