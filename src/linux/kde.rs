//! The look of the Plasma desktop, read from `kdeglobals` and the color scheme.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::gfx::Color;
use crate::theme::{BREEZE_DARK, BREEZE_LIGHT, DesktopColors, Kind, SystemLook};

/// A KDE config file: `[Group]` → key → value.
type Ini = HashMap<String, HashMap<String, String>>;

pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default()
}

fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    home.into_iter().chain(system.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)).collect()
}

fn kdeglobals() -> PathBuf {
    config_home().join("kdeglobals")
}

/// When `kdeglobals` last changed, to notice a new color scheme.
pub fn modified() -> Option<SystemTime> {
    std::fs::metadata(kdeglobals()).and_then(|m| m.modified()).ok()
}

fn parse_ini(text: &str) -> Ini {
    let mut ini = Ini::new();
    let mut group = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            // Nested groups are written [Colors:Window][Inactive]; keep the whole name.
            group = line.replace("][", "/").trim_matches(['[', ']']).to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            // Drop KDE's [$e] / [$i] markers.
            let k = k.split('[').next().unwrap_or(k).trim();
            ini.entry(group.clone()).or_default().insert(k.to_string(), v.trim().to_string());
        }
    }
    ini
}

fn read_ini(path: &PathBuf) -> Option<Ini> {
    std::fs::read_to_string(path).ok().map(|t| parse_ini(&t))
}

/// `r,g,b` or `r,g,b,a` (0-255), or `#rrggbb`.
fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let n = u32::from_str_radix(hex, 16).ok()?;
        if hex.len() != 6 {
            return None;
        }
        return Some(crate::theme::rgba((n >> 16) as u8, (n >> 8) as u8, n as u8, 255));
    }
    let parts: Vec<u8> = v.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    match parts[..] {
        [r, g, b] => Some(crate::theme::rgba(r, g, b, 255)),
        [r, g, b, a] => Some(crate::theme::rgba(r, g, b, a)),
        _ => None,
    }
}

fn color(ini: &Ini, group: &str, key: &str) -> Option<Color> {
    ini.get(group)?.get(key).and_then(|v| parse_color(v))
}

/// Read the colors from `ini`, taking what is missing from `base`.
fn colors_from(ini: &Ini, base: DesktopColors) -> DesktopColors {
    DesktopColors {
        window: color(ini, "Colors:Window", "BackgroundNormal").unwrap_or(base.window),
        text: color(ini, "Colors:Window", "ForegroundNormal").unwrap_or(base.text),
        inactive: color(ini, "Colors:Window", "ForegroundInactive").unwrap_or(base.inactive),
        view: color(ini, "Colors:View", "BackgroundNormal").unwrap_or(base.view),
        highlight: color(ini, "General", "AccentColor")
            .or_else(|| color(ini, "Colors:Selection", "BackgroundNormal"))
            .unwrap_or(base.highlight),
    }
}

/// The installed color scheme file named `name` (`BreezeDark` → `BreezeDark.colors`).
fn scheme_file(name: &str) -> Option<PathBuf> {
    let file = format!("{name}.colors");
    data_dirs().into_iter().map(|d| d.join("color-schemes").join(&file)).find(|p| p.exists())
}

/// The desktop colors. Plasma copies the scheme into `kdeglobals`; when a group is
/// missing there, the scheme file named by `ColorScheme` fills it in.
pub fn desktop_colors(globals: &Ini) -> DesktopColors {
    let scheme = globals.get("General").and_then(|g| g.get("ColorScheme")).cloned().unwrap_or_default();
    let fallback = if scheme.to_ascii_lowercase().contains("dark") { BREEZE_DARK } else { BREEZE_LIGHT };
    let base = scheme_file(&scheme).and_then(|p| read_ini(&p)).map(|s| colors_from(&s, fallback)).unwrap_or(fallback);
    colors_from(globals, base)
}

/// The family of `font=Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1`.
fn font_family(globals: &Ini) -> Option<String> {
    let v = globals.get("General")?.get("font")?;
    let family = v.split(',').next()?.trim();
    (!family.is_empty()).then(|| family.to_string())
}

/// How Plasma looks right now. `blur`: the compositor blurs behind translucent surfaces.
pub fn system_look(blur: bool) -> SystemLook {
    let globals = read_ini(&kdeglobals()).unwrap_or_default();
    let desktop = desktop_colors(&globals);
    SystemLook {
        auto_kind: Kind::Kde,
        dark: desktop.dark(),
        accent: Some((desktop.highlight, desktop.highlight)),
        transparency: true,
        high_contrast: false,
        system_backdrop: false,
        blur,
        desktop: Some(desktop),
        font: font_family(&globals),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_kdeglobals() {
        let ini = parse_ini(
            "[General]\nColorScheme=Foo\nfont=Inter,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\n\
             [Colors:Window]\nBackgroundNormal=32,35,38\nForegroundNormal=252,252,252\n\
             [Colors:Window][Inactive]\nBackgroundNormal=1,2,3\n\
             [Colors:Selection]\nBackgroundNormal=61,174,233\n[WM]\nactiveBackground[$e]=#102030\n",
        );
        assert_eq!(font_family(&ini).as_deref(), Some("Inter"));
        let c = colors_from(&ini, BREEZE_LIGHT);
        assert_eq!(c.window, crate::theme::rgba(32, 35, 38, 255));
        assert_eq!(c.highlight, crate::theme::rgba(61, 174, 233, 255));
        // Missing entries come from the base scheme.
        assert_eq!(c.view, BREEZE_LIGHT.view);
        assert!(c.dark());
        assert_eq!(ini["Colors:Window/Inactive"]["BackgroundNormal"], "1,2,3");
        assert_eq!(ini["WM"]["activeBackground"], "#102030");
        assert_eq!(parse_color("#ff8000"), Some(crate::theme::rgba(255, 128, 0, 255)));
    }
}
