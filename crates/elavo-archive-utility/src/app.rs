//! One Archive Utility window per session, with the system light and dark setting.

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
        crate::accent::install();
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

fn show(app: &gtk::Application, paths: Vec<PathBuf>, autostart: bool) {
    let archives: Vec<PathBuf> = paths
        .iter()
        .filter(|path| elavo_archive_utility_core::detect(path).is_some())
        .cloned()
        .collect();
    if !archives.is_empty() && archives.len() == paths.len() {
        crate::extract::open(app, archives);
        return;
    }
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
    match std::env::var("ELAVO_ARCHIVE_UTILITY_NO_AUTOSTART") {
        Ok(value) if value != "0" && !value.is_empty() => false,
        _ => true,
    }
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
/// on either desktop being installed.
fn follow_color_scheme() {
    if let Some(gtk_settings) = gtk::Settings::default() {
        gtk_settings.set_property("gtk-application-prefer-dark-theme", true);
        // Elavo keeps the window buttons on the left.
        gtk_settings.set_gtk_decoration_layout(Some("close,minimize,maximize:"));
    }
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
    let apply = |_iface: &gio::Settings| {
        if let Some(gtk_settings) = gtk::Settings::default() {
            gtk_settings.set_property("gtk-application-prefer-dark-theme", true);
            gtk_settings.set_gtk_decoration_layout(Some("close,minimize,maximize:"));
        }
    };
    apply(&iface);
    iface.connect_changed(Some("color-scheme"), move |settings, _| apply(settings));
    std::mem::forget(iface);
}
