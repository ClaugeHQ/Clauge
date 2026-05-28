use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceConfig {
    pub theme: String,
    pub accent_color: String,
    /// CSS `font-family` value applied to xterm.js terminals. Empty means
    /// "use the built-in default stack" on the renderer side — see
    /// `DEFAULT_TERMINAL_FONT_FAMILY` in
    /// `src/lib/shared/primitives/terminal-utils.ts`. Stored verbatim in the
    /// `settings` table under the `terminal_font_family` key.
    #[serde(default)]
    pub terminal_font_family: String,
}

/// Default theme per OS:
/// - macOS / Windows: dark-glass (vibrancy / mica)
/// - Linux: dark-solid (no reliable cross-distro translucency)
fn default_theme() -> String {
    if cfg!(target_os = "linux") {
        "dark-solid".to_string()
    } else {
        "dark-glass".to_string()
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            accent_color: "#6366f1".to_string(),
            terminal_font_family: String::new(),
        }
    }
}

#[cfg(target_os = "macos")]
pub fn apply_vibrancy(window: &tauri::WebviewWindow, material: &str) -> Result<(), String> {
    use window_vibrancy::{apply_vibrancy as wv_apply, NSVisualEffectMaterial};

    let mat = match material {
        "titlebar" => NSVisualEffectMaterial::Titlebar,
        "sidebar" => NSVisualEffectMaterial::Sidebar,
        "under-window" => NSVisualEffectMaterial::UnderWindowBackground,
        "hud" => NSVisualEffectMaterial::HudWindow,
        "content" => NSVisualEffectMaterial::ContentBackground,
        "header" => NSVisualEffectMaterial::HeaderView,
        "window" => NSVisualEffectMaterial::WindowBackground,
        "menu" => NSVisualEffectMaterial::Menu,
        "popover" => NSVisualEffectMaterial::Popover,
        "selection" => NSVisualEffectMaterial::Selection,
        #[allow(deprecated)]
        "dark" => NSVisualEffectMaterial::Dark,
        #[allow(deprecated)]
        "ultra-dark" => NSVisualEffectMaterial::UltraDark,
        "none" => return Ok(()),
        _ => NSVisualEffectMaterial::Sidebar,
    };

    wv_apply(window, mat, None, None).map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
pub fn apply_vibrancy(window: &tauri::WebviewWindow, material: &str) -> Result<(), String> {
    use window_vibrancy::{apply_acrylic, apply_mica, clear_acrylic, clear_mica};

    if material == "none" {
        // Best-effort clear: ignore errors if no effect was applied yet.
        let _ = clear_mica(window);
        let _ = clear_acrylic(window);
        return Ok(());
    }

    // Prefer Mica (Win11). Fall back to Acrylic (Win10) if Mica isn't available.
    if apply_mica(window, Some(true)).is_ok() {
        return Ok(());
    }
    apply_acrylic(window, Some((18, 18, 18, 125))).map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
pub fn apply_vibrancy(_window: &tauri::WebviewWindow, material: &str) -> Result<(), String> {
    // Linux has no cross-distro vibrancy API. The glass effect comes
    // from `transparent: true` (set in `tauri.linux.conf.json`) plus
    // the compositor's own blur — when the compositor supports it.
    // We return Ok ONLY when the running desktop is in our known
    // blur-capable allowlist; everywhere else we Err so the CSS
    // fallback (opaque body background, see app.css `body.glass-mode`)
    // kicks in and the user can't end up with a fully-transparent
    // window showing the white WebKitGTK default through it.
    if material == "none" {
        return Ok(());
    }
    if linux_compositor_supports_blur() {
        Ok(())
    } else {
        Err("compositor blur unavailable".to_string())
    }
}

/// Allowlist of Linux desktops where window blur is reliable enough
/// to enable the glass theme by default. The check is based on
/// `XDG_CURRENT_DESKTOP` (and `DESKTOP_SESSION` as a secondary
/// signal) — both are set by every modern session manager.
///
/// Known good:
///   • KDE Plasma — KWin's blur effect is on by default since Plasma 5,
///     polished in Plasma 6.
///   • Hyprland — has a first-class blur shader; on by default.
///   • Sway — supports blur via the `blur` directive (most users have
///     it on if they bother with Sway theming).
///   • GNOME — Mutter doesn't blur by default, but a large share of
///     GNOME users run the "Blur My Shell" extension. Including it
///     here because the rest of the chrome (transparent window,
///     rgba surfaces) still degrades gracefully without blur.
///
/// Explicitly NOT here: XFCE, MATE, LXQt, Cinnamon, Pantheon,
/// Enlightenment, IceWM — these either don't composite or compositor
/// blur isn't a settled feature. Users on these can still pick the
/// theme; they just get the safe opaque fallback CSS rather than a
/// risky transparent-window-with-no-blur look.
#[cfg(target_os = "linux")]
fn linux_compositor_supports_blur() -> bool {
    let de = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let session = std::env::var("DESKTOP_SESSION")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let candidates = [&de, &session];
    candidates.iter().any(|s| {
        s.contains("kde")
            || s.contains("plasma")
            || s.contains("hyprland")
            || s.contains("sway")
            || s.contains("gnome")
    })
}

#[tauri::command]
pub async fn set_vibrancy(window: tauri::WebviewWindow, material: String) -> Result<(), String> {
    apply_vibrancy(&window, &material)
}

#[tauri::command]
pub async fn get_appearance(pool: State<'_, SqlitePool>) -> Result<AppearanceConfig, String> {
    let mut config = AppearanceConfig::default();

    if let Ok(Some(row)) =
        sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = 'theme'")
            .fetch_optional(pool.inner())
            .await
    {
        config.theme = row.0;
    }

    if let Ok(Some(row)) =
        sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = 'accent_color'")
            .fetch_optional(pool.inner())
            .await
    {
        config.accent_color = row.0;
    }

    if let Ok(Some(row)) = sqlx::query_as::<_, (String,)>(
        "SELECT value FROM settings WHERE key = 'terminal_font_family'",
    )
    .fetch_optional(pool.inner())
    .await
    {
        config.terminal_font_family = row.0;
    }

    Ok(config)
}

#[tauri::command]
pub async fn set_appearance(
    pool: State<'_, SqlitePool>,
    window: tauri::WebviewWindow,
    config: AppearanceConfig,
) -> Result<(), String> {
    let is_glass = config.theme == "dark-glass";
    // Sidebar material on macOS: brighter than HudWindow (which
    // renders intentionally dark for floating heads-up displays) and
    // the recommended material for main window chrome. Combined with
    // the lower CSS surface alphas (0.62) the wallpaper still reads
    // through clearly — without darkening the overall feel. On
    // Windows / Linux the material arg is ignored: Mica / Acrylic /
    // compositor transparency drive the look there.
    let vibrancy_material = if is_glass { "sidebar" } else { "none" };

    let settings = [
        ("theme", config.theme.as_str()),
        ("accent_color", config.accent_color.as_str()),
        ("terminal_font_family", config.terminal_font_family.as_str()),
    ];

    for (key, value) in &settings {
        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(pool.inner())
            .await
            .map_err(|e| e.to_string())?;
    }

    // On Linux, vibrancy is unsupported — silently swallow the error so the
    // user can still pick "dark-glass" without seeing a broken-looking failure.
    // The theme just renders as opaque dark.
    let _ = apply_vibrancy(&window, vibrancy_material);
    Ok(())
}

/// Get list of available themes (filtered per-OS).
#[tauri::command]
pub async fn get_available_themes() -> Result<Vec<ThemeInfo>, String> {
    let all = vec![
        ThemeInfo {
            id: "dark-glass".to_string(),
            name: "Dark Glass".to_string(),
            description: "Translucent with native blur".to_string(),
            preview_bg: "#07070f".to_string(),
            preview_accent: "#7c5cf8".to_string(),
        },
        ThemeInfo {
            id: "dark-solid".to_string(),
            name: "Dark Solid".to_string(),
            description: "Solid dark surfaces, no transparency".to_string(),
            preview_bg: "#0d0d18".to_string(),
            preview_accent: "#7c5cf8".to_string(),
        },
        ThemeInfo {
            id: "midnight".to_string(),
            name: "Midnight".to_string(),
            description: "Deep blacks, high contrast, OLED-friendly".to_string(),
            preview_bg: "#000000".to_string(),
            preview_accent: "#4f94d4".to_string(),
        },
        ThemeInfo {
            id: "rose-pine-moon".to_string(),
            name: "Rose Pine Moon".to_string(),
            description: "Warm pastel pinks and lavenders".to_string(),
            preview_bg: "#232136".to_string(),
            preview_accent: "#c4a7e7".to_string(),
        },
        ThemeInfo {
            id: "rose-pine-dawn".to_string(),
            name: "Rose Pine Dawn".to_string(),
            description: "Warm cream light — Rose Pine sister to Moon".to_string(),
            preview_bg: "#faf4ed".to_string(),
            preview_accent: "#907aa9".to_string(),
        },
        // Atelier — premium theme. `premium: true` lives in the JS registry
        // (theme.ts), which the picker reads to render the lock + paywall.
        // The pixel-critter parade is rendered by
        // src/lib/components/effects/CatsParade.svelte and gated to the
        // active appearance.theme — non-Pro previewers see the palette
        // without the critters, which is the upgrade carrot. The botanical
        // wallpaper lives on `body.fx-atelier` (see app.css).
        ThemeInfo {
            id: "atelier".to_string(),
            name: "Atelier".to_string(),
            description: "Blush + botanical wallpaper — pixel critters wander the footer".to_string(),
            preview_bg: "#fdf2f5".to_string(),
            preview_accent: "#b8395a".to_string(),
        },
        // Petal — premium theme. `premium: true` lives in the JS registry
        // (theme.ts). Falling cherry-blossom petals (PetalFall.svelte) +
        // plum-violet palette. Gated on appearance.theme so non-Pro
        // previewers see the palette but no petals.
        ThemeInfo {
            id: "petal".to_string(),
            name: "Petal".to_string(),
            description: "Plum twilight with falling cherry petals".to_string(),
            preview_bg: "#14091e".to_string(),
            preview_accent: "#f4a5b8".to_string(),
        },
        // Celeste — premium theme. Cosmic violet-black with twinkling
        // stars + occasional shooting stars (Starfield.svelte). Gated on
        // appearance.theme so non-Pro previewers see the palette but no
        // starfield.
        ThemeInfo {
            id: "celeste".to_string(),
            name: "Celeste".to_string(),
            description: "Cosmic violet-black with twinkling stars".to_string(),
            preview_bg: "#0a0814".to_string(),
            preview_accent: "#a8b8e8".to_string(),
        },
        // Hearth — premium theme. `premium: true` lives in the JS registry
        // (theme.ts). The animated ember swarm is rendered by
        // src/lib/components/effects/Embers.svelte and gated to the active
        // appearance.theme — non-Pro previewers see the warm palette but
        // no embers (the embers are the upgrade carrot).
        ThemeInfo {
            id: "hearth".to_string(),
            name: "Hearth".to_string(),
            description: "Warm charcoal with rising embers — animated firelight".to_string(),
            preview_bg: "#14100c".to_string(),
            preview_accent: "#ff8c42".to_string(),
        },
        // `aurora-drift`, `carbon-grain`, and `crt-phosphor` are intentionally
        // not registered here — they don't appear in the picker. Their JS
        // theme entries, terminal palettes, preview swatches, and CSS rules
        // are kept in place so anyone with one of these IDs already stored
        // in their settings still gets it applied correctly via `applyTheme`,
        // and so the entries can be re-surfaced later without rebuilding
        // the palettes.
    ];

    // dark-glass is now reachable on every OS — macOS via
    // NSVisualEffectMaterial::HudWindow, Windows via Mica/Acrylic,
    // Linux via `transparent: true` + compositor blur. The Linux
    // first-launch default is still dark-solid (see `default_theme`)
    // because compositor blur isn't guaranteed; user opts in.
    Ok(all)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub preview_bg: String,
    pub preview_accent: String,
}
