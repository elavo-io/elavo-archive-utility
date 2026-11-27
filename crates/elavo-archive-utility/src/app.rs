//! One Archive Utility window per session. Colors, window frames, and the
//! accent come from the GTK theme; the app follows the system light and dark
//! setting the way Elavo Files does.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::window::{wire_extra, Window};

pub const APP_ID: &str = "io.elavo.ArchiveUtility";

thread_local! {
    static WINDOW: Cell<Option<Rc<Window>>> = const { Cell::new(None) };
    static OPEN_COMPACT: Cell<bool> = const { Cell::new(false) };
}

pub fn run() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN | gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    app.connect_startup(|_| {
        load_css();
        install_icon();
        follow_color_scheme();
        gtk::Window::set_default_icon_name(APP_ID);
    });
    app.connect_activate(|app| {
        match std::env::var("ELAVO_ARCHIVE_UTILITY_SCREENSHOT")
            .ok()
            .as_deref()
        {
            Some("extract") => crate::extract::demo(app, 1),
            Some("extract-two") => crate::extract::demo(app, 4),
            _ => show(app, Vec::new(), false),
        }
    });
    app.connect_open(|app, files, _| {
        let paths = files.iter().filter_map(|file| file.path()).collect();
        show(app, paths, autostart());
    });
    app.connect_command_line(|app, command| {
        let args = command.arguments();
        let mut compact = false;
        let mut paths = Vec::new();
        for arg in args.iter().skip(1) {
            let text = arg.to_string_lossy();
            if text == "--compact" {
                compact = true;
                continue;
            }
            if text.starts_with('-') {
                continue;
            }
            let path = PathBuf::from(text.as_ref());
            let path = if path.is_absolute() {
                path
            } else if let Some(cwd) = command.cwd() {
                cwd.join(path)
            } else {
                path
            };
            paths.push(path);
        }
        OPEN_COMPACT.with(|cell| cell.set(compact));
        if paths.is_empty() {
            app.activate();
        } else {
            app.open(
                &paths.iter().map(gio::File::for_path).collect::<Vec<_>>(),
                "",
            );
        }
        0
    });

    app.set_accels_for_action("win.preferences", &["<Control>comma"]);
    app.set_accels_for_action("win.start", &["<Control>Return"]);
    app.set_accels_for_action("win.cancel", &["Escape"]);
    app.set_accels_for_action("win.close", &["<Control>w"]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    let quit = gio::SimpleAction::new("quit", None);
    let app_quit = app.downgrade();
    quit.connect_activate(move |_, _| {
        if let Some(app) = app_quit.upgrade() {
            app.quit();
        }
    });
    app.add_action(&quit);

    app.run()
}

/// Opening files is a request to extract them. Every file goes to the
/// extraction window: recognised archives with their own tool, anything
/// else through 7-Zip, which says on the file's row when it cannot open it.
/// Folders cannot be extracted, so folders in the same open are compressed
/// in the main window, as before.
fn show(app: &gtk::Application, paths: Vec<PathBuf>, autostart: bool) {
    let (folders, files): (Vec<PathBuf>, Vec<PathBuf>) =
        paths.into_iter().partition(|path| path.is_dir());
    if !files.is_empty() {
        crate::extract::open(app, files);
        if folders.is_empty() {
            return;
        }
    }
    let paths = folders;
    let compact = OPEN_COMPACT.with(|cell| cell.get());
    let existing = WINDOW.with(|cell| cell.take());
    let window = if let Some(window) = existing {
        WINDOW.with(|cell| cell.set(Some(window.clone())));
        window
    } else {
        let window = Window::new(app, compact);
        wire_extra(&window);
        let app_for_close = app.clone();
        // The window registers itself; quitting the app when it closes is the
        // default for a single window because it is the only one.
        let _ = app_for_close;
        WINDOW.with(|cell| cell.set(Some(window.clone())));
        window
    };
    if !paths.is_empty() {
        window.offer(paths, autostart);
    }
    window.present();
}

fn autostart() -> bool {
    !matches!(
        std::env::var("ELAVO_ARCHIVE_UTILITY_NO_AUTOSTART"),
        Ok(value) if value != "0" && !value.is_empty()
    )
}

fn load_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let css = gtk::CssProvider::new();
    css.load_from_resource("/io/elavo/ArchiveUtility/resources/style.css");
    gtk::style_context_add_provider_for_display(
        &display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn install_icon() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&display);
    theme.add_resource_path("/io/elavo/ArchiveUtility/icons");
    // Flatpak hides the host /usr. It bind-mounts icon themes at these two
    // paths without an extra permission. Putting them first lets Papirus,
    // Humanity, or an Elavo theme win over the runtime's Adwaita copy.
    // ~/.icons stays visible because the sandbox can read home.
    if std::path::Path::new("/.flatpak-info").is_file() {
        let mut paths = Vec::new();
        for extra in ["/run/host/user-share/icons", "/run/host/share/icons"] {
            if std::path::Path::new(extra).is_dir() {
                paths.push(std::path::PathBuf::from(extra));
            }
        }
        paths.extend(theme.search_path());
        let refs: Vec<&std::path::Path> = paths.iter().map(|path| path.as_path()).collect();
        theme.set_search_path(&refs);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            let icons = root.join("data/icons");
            if icons.is_dir() {
                theme.add_search_path(&icons);
            }
        }
    }
}

/// Follow the same color-scheme key GNOME and Elavo write, without depending
/// on either desktop being installed. The button layout is the system's
/// (gtk-decoration-layout), so it is not set here.
fn follow_color_scheme() {
    let Some(source) = gio::SettingsSchemaSource::default() else {
        return;
    };
    let Some(schema) = source.lookup("org.gnome.desktop.interface", true) else {
        return;
    };
    if !schema.has_key("color-scheme") {
        return;
    }
    let iface = gio::Settings::new("org.gnome.desktop.interface");
    let apply = |iface: &gio::Settings| {
        if let Some(gtk_settings) = gtk::Settings::default() {
            let dark = iface.string("color-scheme") == "prefer-dark";
            set_gtk_dark(&gtk_settings, dark);
        }
    };
    apply(&iface);
    iface.connect_changed(Some("color-scheme"), move |settings, _| apply(settings));
    std::mem::forget(iface);
}

/// Load the light or dark sheet for `dark`. GTK 4.24 picks the sheet from
/// `gtk-interface-color-scheme`, and `gtk-application-prefer-dark-theme`
/// reloads the sheet before it updates that value, so one call alone loads
/// the sheet of the previous scheme. Set the scheme first where GTK has it,
/// then set the old property twice so the reload sees the new scheme. The
/// theme name is left alone so accent changes still reach the app.
fn set_gtk_dark(settings: &gtk::Settings, dark: bool) {
    let scheme = settings
        .find_property("gtk-interface-color-scheme")
        .and_then(|pspec| {
            let class = gtk::glib::EnumClass::with_type(pspec.value_type())?;
            class.to_value_by_nick(if dark { "dark" } else { "light" })
        });
    if let Some(value) = &scheme {
        settings.set_property_from_value("gtk-interface-color-scheme", value);
    }
    settings.set_gtk_application_prefer_dark_theme(dark);
    settings.set_gtk_application_prefer_dark_theme(dark);
}
