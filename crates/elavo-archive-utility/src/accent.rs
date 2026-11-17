//! The accent used by progress bars, the level slider, checks, and selection.
//!
//! Libadwaita and GNOME 47 publish the color through the settings portal
//! (`org.freedesktop.appearance` / `accent-color`, three sRGB components).
//! Elavo uses the same GNOME setting name and, because its palette is not
//! Adwaita's, also writes `~/.config/gtk-4.0/elavo-accent.css` with
//! `--elavo-user-accent: var(--elavo-accent-<name>)`. That file wins when it
//! names a preset. Otherwise the portal color is used, then the GNOME
//! setting, then Elavo blue.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

/// Elavo's presets from `elavo-visuals` `_elavo-accents.scss`. The `gnome`
/// column is `org.gnome.desktop.interface accent-color`, which libadwaita
/// also reads.
const PRESETS: &[(&str, &str, &str)] = &[
    ("blue", "#0860f2", "#ffffff"),
    ("purple", "#8a4fd8", "#ffffff"),
    ("pink", "#d6337f", "#ffffff"),
    ("red", "#d93b36", "#ffffff"),
    ("orange", "#c95a06", "#ffffff"),
    ("yellow", "#f2c230", "#1c1c1c"),
    ("green", "#258a3b", "#ffffff"),
    ("graphite", "#6e6e73", "#ffffff"),
    ("slate", "#6e6e73", "#ffffff"),
];

const FALLBACK: (&str, &str) = ("#0860f2", "#ffffff");

pub fn install() {
    let (bg, fg) = resolve();
    paint(&bg, &fg);
    watch();
}

fn resolve() -> (String, String) {
    if let Some(color) = std::env::var("ELAVO_ARCHIVE_UTILITY_ACCENT")
        .ok()
        .and_then(|text| parse_hex(&text))
    {
        return color;
    }
    if let Some(color) = elavo_file() {
        return color;
    }
    if let Some(color) = portal_color() {
        return color;
    }
    if let Some(color) = gnome_setting() {
        return color;
    }
    (FALLBACK.0.to_string(), FALLBACK.1.to_string())
}

fn paint(bg: &str, fg: &str) {
    use std::cell::RefCell;
    thread_local! {
        static PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    }
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let sheet = format!(
        "@define-color accent_color {bg};\n@define-color accent_bg_color {bg};\n@define-color accent_fg_color {fg};\n"
    );
    PROVIDER.with(|slot| {
        let existing = slot.borrow().clone();
        let provider = if let Some(provider) = existing {
            provider
        } else {
            let provider = gtk::CssProvider::new();
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
            *slot.borrow_mut() = Some(provider.clone());
            provider
        };
        provider.load_from_string(&sheet);
    });
}

fn watch() {
    if let Some(settings) = gnome_settings() {
        settings.connect_changed(Some("accent-color"), |_, _| {
            let (bg, fg) = resolve();
            paint(&bg, &fg);
        });
        std::mem::forget(settings);
    }
    let path = elavo_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = gio::File::for_path(&path);
    if let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
        monitor.connect_changed(|_, _, _, _| {
            let (bg, fg) = resolve();
            paint(&bg, &fg);
        });
        std::mem::forget(monitor);
    }
}

fn elavo_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("gtk-4.0").join("elavo-accent.css")
}

fn elavo_file() -> Option<(String, String)> {
    let text = std::fs::read_to_string(elavo_path()).ok()?;
    parse_elavo_css(&text)
}

/// `--elavo-user-accent: var(--elavo-accent-orange)` or a direct hex color.
pub fn parse_elavo_css(css: &str) -> Option<(String, String)> {
    let lower = css.to_ascii_lowercase();
    if let Some(name) = between(&lower, "--elavo-user-accent: var(--elavo-accent-", ")") {
        return preset(name);
    }
    if let Some(hex) = between(&lower, "--accent-bg-color:", ";") {
        let hex = hex.trim().trim_matches('"');
        if let Some(color) = parse_hex(hex) {
            return Some(color);
        }
    }
    if lower.contains("--elavo-accent-blue") {
        return Some((FALLBACK.0.to_string(), FALLBACK.1.to_string()));
    }
    None
}

fn gnome_setting() -> Option<(String, String)> {
    let settings = gnome_settings()?;
    let name = settings.string("accent-color");
    preset(name.as_str())
}

fn gnome_settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    let schema = source.lookup("org.gnome.desktop.interface", true)?;
    if !schema.has_key("accent-color") {
        return None;
    }
    Some(gio::Settings::new("org.gnome.desktop.interface"))
}

fn portal_color() -> Option<(String, String)> {
    let proxy = gio::DBusProxy::for_bus_sync(
        gio::BusType::Session,
        gio::DBusProxyFlags::NONE,
        None,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        gio::Cancellable::NONE,
    )
    .ok()?;
    let reply = proxy
        .call_sync(
            "Read",
            Some(&("org.freedesktop.appearance", "accent-color").to_variant()),
            gio::DBusCallFlags::NONE,
            800,
            gio::Cancellable::NONE,
        )
        .ok()?;
    let (r, g, b) = rgb_from_variant(&reply)?;
    Some(rgb_to_hex(r, g, b))
}

fn rgb_from_variant(value: &glib::Variant) -> Option<(f64, f64, f64)> {
    let mut current = value.clone();
    for _ in 0..4 {
        if current.type_().is_variant() {
            current = current.child_value(0);
            continue;
        }
        break;
    }
    if current.n_children() != 3 {
        return None;
    }
    Some((
        current.child_value(0).get::<f64>()?,
        current.child_value(1).get::<f64>()?,
        current.child_value(2).get::<f64>()?,
    ))
}

fn rgb_to_hex(r: f64, g: f64, b: f64) -> (String, String) {
    let channel = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let bg = format!("#{:02x}{:02x}{:02x}", channel(r), channel(g), channel(b));
    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let fg = if luma > 0.62 { "#1c1c1c" } else { "#ffffff" };
    (bg, fg.to_string())
}

pub fn preset(name: &str) -> Option<(String, String)> {
    PRESETS
        .iter()
        .find(|(id, _, _)| *id == name)
        .map(|(_, bg, fg)| ((*bg).to_string(), (*fg).to_string()))
}

pub fn parse_hex(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let r = ((value >> 16) & 0xff) as f64 / 255.0;
    let g = ((value >> 8) & 0xff) as f64 / 255.0;
    let b = (value & 0xff) as f64 / 255.0;
    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let fg = if luma > 0.62 { "#1c1c1c" } else { "#ffffff" };
    Some((format!("#{hex}"), fg.to_string()))
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = text.split(start).nth(1)?;
    Some(rest.split(end).next()?.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elavo_named_accent_uses_elavos_hex() {
        let css = ":root,\ntoolbarview {\n  --elavo-user-accent: var(--elavo-accent-orange);\n  --elavo-user-accent-fg: var(--elavo-accent-orange-fg);\n}\n";
        assert_eq!(
            parse_elavo_css(css),
            Some(("#c95a06".into(), "#ffffff".into()))
        );
        assert_eq!(preset("slate"), Some(("#6e6e73".into(), "#ffffff".into())));
        assert_eq!(preset("missing"), None);
    }

    #[test]
    fn elavo_blue_file_is_the_fallback() {
        let css =
            ":root { --accent-bg-color: var(--elavo-user-accent, var(--elavo-accent-blue)); }";
        assert_eq!(
            parse_elavo_css(css),
            Some(("#0860f2".into(), "#ffffff".into()))
        );
    }

    #[test]
    fn yellow_text_is_dark() {
        assert_eq!(preset("yellow"), Some(("#f2c230".into(), "#1c1c1c".into())));
        assert_eq!(parse_hex("#f2c230").unwrap().1, "#1c1c1c");
        assert_eq!(parse_hex("0860f2").unwrap().0, "#0860f2");
    }
}
