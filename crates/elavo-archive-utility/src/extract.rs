//! The window used when a file manager opens an archive.
//!
//! One row per archive. Later launches of the same instance add rows.
//! The window closes, and the process quits if the main window is not open,
//! once every row has finished without an error.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use elavo_archive_utility_core::{
    suggest_extract_parent, Cancel, Error, ExtractRequest, Outcome, Progress, Settings,
};

struct Row {
    id: u64,
    archive: PathBuf,
    cancel: Cancel,
    started: Instant,
    paused: Cell<bool>,
    failed: Cell<bool>,
    title: gtk::Label,
    bar: gtk::ProgressBar,
    estimate: gtk::Label,
    pause: gtk::Button,
    card: gtk::Box,
}

enum Note {
    Progress {
        id: u64,
        fraction: Option<f64>,
        #[allow(dead_code)]
        message: String,
    },
    Finished {
        id: u64,
        result: std::result::Result<Outcome, String>,
        needs_password: bool,
    },
}

pub struct Extractor {
    window: gtk::ApplicationWindow,
    list: gtk::Box,
    rows: RefCell<Vec<Rc<Row>>>,
    next_id: Cell<u64>,
    tx: Sender<Note>,
    asking: Cell<bool>,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<Extractor>>> = const { RefCell::new(None) };
}

pub fn open(app: &gtk::Application, archives: Vec<PathBuf>) {
    let extractor = current(app);
    for archive in archives {
        extractor.start(archive, None);
    }
    extractor.window.present();
}

pub fn demo(app: &gtk::Application, jobs: usize) {
    let extractor = current(app);
    // Real magic bytes, so Gio sniffs a content type instead of an empty file.
    let samples: &[(&str, &[u8], f64, &str)] = &[
        (
            "notes.zip",
            b"PK\x03\x04\x14\x00\x00\x00",
            0.62,
            "About 8 seconds",
        ),
        (
            "photos.7z",
            &[0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c, 0x00, 0x04],
            0.28,
            "About 20 seconds",
        ),
        (
            "disk.dmg",
            b"\0\0\0\0koly\0\0\0\0disk image",
            0.45,
            "About 12 seconds",
        ),
        (
            "setup.exe",
            b"MZ\x90\x00\x03\x00\x00\x00\x04\x00",
            0.18,
            "About 40 seconds",
        ),
    ];
    let dir = std::env::temp_dir().join("au-shot-archives");
    let _ = std::fs::create_dir_all(&dir);
    for (name, bytes, fraction, estimate) in samples.iter().take(jobs) {
        let path = dir.join(name);
        let _ = std::fs::write(&path, bytes);
        let row = extractor.add_row(&path);
        row.bar.set_fraction(*fraction);
        row.estimate.set_label(estimate);
        row.title.set_label(&format!("Extracting \"{name}\""));
    }
    extractor.window.present();
}

fn current(app: &gtk::Application) -> Rc<Extractor> {
    if let Some(extractor) = OPEN.with(|slot| slot.borrow().clone()) {
        return extractor;
    }
    let extractor = Extractor::new(app);
    OPEN.with(|slot| *slot.borrow_mut() = Some(extractor.clone()));
    extractor
}

impl Extractor {
    fn new(app: &gtk::Application) -> Rc<Self> {
        // A plain window with the theme's title bar, like the File
        // Operations window in Elavo Files.
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Extracting")
            .default_width(460)
            .resizable(false)
            .css_classes(["background", "au", "au-extract-window"])
            .build();
        window.set_titlebar(Some(&gtk::HeaderBar::new()));
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .css_classes(["au-jobs"])
            .build();
        window.set_child(Some(&list));
        let (tx, rx) = mpsc::channel();
        let extractor = Rc::new(Self {
            window,
            list,
            rows: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
            tx,
            asking: Cell::new(false),
        });
        let inbox = Rc::new(RefCell::new(rx));
        let weak = Rc::downgrade(&extractor);
        glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
            if let Some(extractor) = weak.upgrade() {
                extractor.drain(&inbox.borrow());
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
        let weak = Rc::downgrade(&extractor);
        extractor.window.connect_close_request(move |_| {
            if let Some(extractor) = weak.upgrade() {
                for row in extractor.rows.borrow().iter() {
                    row.cancel.cancel();
                }
            }
            OPEN.with(|slot| slot.borrow_mut().take());
            glib::Propagation::Proceed
        });
        extractor
    }

    fn start(self: &Rc<Self>, archive: PathBuf, password: Option<String>) {
        let row = self.add_row(&archive);
        let id = row.id;
        let cancel = row.cancel.clone();
        let tx = self.tx.clone();
        // The process stays up until the worker has returned, even when the
        // row is cancelled and the window closes, so the job's temporary
        // folder is always removed.
        let returned = Arc::new(AtomicBool::new(false));
        if let Some(app) = self.window.application() {
            let mut hold = Some(app.hold());
            let returned = returned.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if returned.load(Ordering::Acquire) {
                    hold.take();
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
        std::thread::spawn(move || {
            let settings = Settings::load();
            let destination = suggest_extract_parent(&archive, &settings.output);
            let req = ExtractRequest {
                archive: archive.clone(),
                destination,
                password,
                new_folder: settings.new_folder,
                conflict: settings.conflict,
            };
            let tx_progress = tx.clone();
            let cancel_for_wait = cancel.clone();
            let result = elavo_archive_utility_core::extract(&req, &cancel, |update: Progress| {
                while cancel_for_wait.is_paused() && !cancel_for_wait.is_cancelled() {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                }
                let _ = tx_progress.send(Note::Progress {
                    id,
                    fraction: update.fraction,
                    message: update.message,
                });
            });
            let needs_password = matches!(
                result,
                Err(Error::PasswordRequired) | Err(Error::WrongPassword)
            );
            let result = result.map_err(|err| err.to_string());
            let _ = tx.send(Note::Finished {
                id,
                result,
                needs_password,
            });
            returned.store(true, Ordering::Release);
        });
    }

    fn add_row(self: &Rc<Self>, archive: &std::path::Path) -> Rc<Row> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let name = archive
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "archive".into());
        let icon = archive_icon(archive);
        let title = gtk::Label::builder()
            .label(format!("Extracting \"{name}\""))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["au-job-title"])
            .hexpand(true)
            .build();
        let pause = round_button("media-playback-pause-symbolic", "Pause");
        let stop = round_button("window-close-symbolic", "Cancel");
        let bar = gtk::ProgressBar::builder()
            .hexpand(true)
            .valign(gtk::Align::Center)
            .show_text(false)
            .build();
        let bar_row = gtk::Box::builder().spacing(8).build();
        bar_row.append(&bar);
        bar_row.append(&pause);
        bar_row.append(&stop);
        let estimate = gtk::Label::builder()
            .label("Estimating…")
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(3)
            .hexpand(true)
            .build();
        text.append(&title);
        text.append(&bar_row);
        text.append(&estimate);
        let card = gtk::Box::builder()
            .spacing(10)
            .css_classes(["au-job"])
            .build();
        card.append(&icon);
        card.append(&text);
        self.list.append(&card);
        let row = Rc::new(Row {
            id,
            archive: archive.to_path_buf(),
            cancel: Cancel::new(),
            started: Instant::now(),
            paused: Cell::new(false),
            failed: Cell::new(false),
            title,
            bar,
            estimate,
            pause,
            card: card.clone(),
        });
        let pause_row = row.clone();
        row.pause.connect_clicked(move |button| {
            if pause_row.paused.get() {
                pause_row.paused.set(false);
                pause_row.cancel.resume();
                button.set_icon_name("media-playback-pause-symbolic");
                button.set_tooltip_text(Some("Pause"));
                pause_row.estimate.set_label("Estimating…");
            } else {
                pause_row.paused.set(true);
                pause_row.cancel.pause();
                button.set_icon_name("media-playback-start-symbolic");
                button.set_tooltip_text(Some("Resume"));
                pause_row.estimate.set_label("Paused");
            }
        });
        let stop_id = id;
        let weak = Rc::downgrade(self);
        stop.connect_clicked(move |_| {
            if let Some(extractor) = weak.upgrade() {
                extractor.cancel_row(stop_id);
            }
        });
        self.rows.borrow_mut().push(row.clone());
        row
    }

    fn cancel_row(&self, id: u64) {
        let mut rows = self.rows.borrow_mut();
        if let Some(index) = rows.iter().position(|row| row.id == id) {
            let row = rows.remove(index);
            row.cancel.cancel();
            self.list.remove(&row.card);
        }
        drop(rows);
        self.maybe_close();
    }

    fn drain(&self, rx: &Receiver<Note>) {
        loop {
            match rx.try_recv() {
                Ok(Note::Progress {
                    id,
                    fraction,
                    message: _,
                }) => self.on_progress(id, fraction),
                Ok(Note::Finished {
                    id,
                    result,
                    needs_password,
                }) => self.on_finished(id, result, needs_password),
                Err(_) => break,
            }
        }
    }

    fn on_progress(&self, id: u64, fraction: Option<f64>) {
        let rows = self.rows.borrow();
        let Some(row) = rows.iter().find(|row| row.id == id) else {
            return;
        };
        if row.paused.get() || row.failed.get() {
            return;
        }
        if let Some(fraction) = fraction {
            let fraction = fraction.clamp(0.0, 1.0);
            row.bar.set_fraction(fraction);
            row.estimate.set_label(&about_remaining(
                row.started.elapsed().as_secs_f64(),
                fraction,
            ));
        } else {
            row.bar.pulse();
        }
    }

    fn on_finished(
        &self,
        id: u64,
        result: std::result::Result<Outcome, String>,
        needs_password: bool,
    ) {
        let row = {
            let rows = self.rows.borrow();
            rows.iter().find(|row| row.id == id).cloned()
        };
        let Some(row) = row else {
            return;
        };
        match result {
            Ok(_) => {
                self.rows.borrow_mut().retain(|item| item.id != id);
                self.list.remove(&row.card);
                self.maybe_close();
            }
            Err(message) if needs_password && !row.cancel.is_cancelled() => {
                self.ask_password(row, message);
            }
            Err(message) => {
                row.failed.set(true);
                row.bar.set_fraction(0.0);
                row.pause.set_visible(false);
                row.estimate.set_label(&message);
                row.estimate.add_css_class("error");
                self.maybe_close();
            }
        }
    }

    fn ask_password(self: &Extractor, row: Rc<Row>, message: String) {
        if self.asking.get() {
            row.failed.set(true);
            row.estimate.set_label(&message);
            return;
        }
        self.asking.set(true);
        let prompt = crate::dialog::password(&self.window, &message);
        let dialog = prompt.window.clone();
        let entry = prompt.entry.clone();
        let open = prompt.open.clone();
        let archive = row.archive.clone();
        let submitted = Rc::new(Cell::new(false));
        let host = OPEN.with(|slot| slot.borrow().clone());
        let dialog_ok = dialog.clone();
        let entry_ok = entry.clone();
        let submitted_ok = submitted.clone();
        let row_for_open = row.clone();
        open.connect_clicked(move |_| {
            submitted_ok.set(true);
            let password = entry_ok.text().to_string();
            if let Some(extractor) = host.clone() {
                extractor.asking.set(false);
                extractor
                    .rows
                    .borrow_mut()
                    .retain(|item| item.id != row_for_open.id);
                extractor.list.remove(&row_for_open.card);
                if !password.is_empty() {
                    extractor.start(archive.clone(), Some(password));
                } else {
                    let failed = extractor.add_row(&archive);
                    failed.failed.set(true);
                    failed.pause.set_visible(false);
                    failed.estimate.set_label("This archive needs a password.");
                    failed.estimate.add_css_class("error");
                }
            }
            dialog_ok.close();
        });
        let row_close = row.clone();
        dialog.connect_close_request(move |_| {
            if let Some(extractor) = OPEN.with(|slot| slot.borrow().clone()) {
                extractor.asking.set(false);
                if !submitted.get() {
                    row_close.failed.set(true);
                    row_close.pause.set_visible(false);
                    row_close
                        .estimate
                        .set_label("This archive needs a password.");
                    row_close.estimate.add_css_class("error");
                }
            }
            glib::Propagation::Proceed
        });
        dialog.present();
        entry.grab_focus();
    }

    /// Close once every row is gone. Rows that failed stay up so their
    /// error can be read; the window closes once they are dismissed.
    fn maybe_close(&self) {
        let empty = self.rows.borrow().is_empty();
        if empty {
            self.window.close();
        }
    }
}

/// The archive's icon from the active theme. The chain asks for the MIME
/// icon of the detected format first (`application-zip`,
/// `application-x-7z-compressed`, …), then Gio's names for the file, then
/// `package-x-generic` and `text-x-generic` as the last resorts.
fn archive_icon(path: &std::path::Path) -> gtk::Image {
    let gicon = icon_for_archive(path);
    if std::env::var_os("ELAVO_ARCHIVE_UTILITY_SCREENSHOT").is_some() {
        report_icon(path, &gicon);
    }
    let icon = gtk::Image::from_gicon(&gicon);
    icon.set_pixel_size(28);
    icon.set_valign(gtk::Align::Center);
    icon.set_halign(gtk::Align::Center);
    icon
}

fn icon_for_archive(path: &std::path::Path) -> gio::Icon {
    let gio_icon = gio_icon_for(path);
    let mut names: Vec<String> = elavo_archive_utility_core::detect(path)
        .map(|format| {
            format
                .icon_names()
                .iter()
                .map(|name| name.to_string())
                .collect()
        })
        .unwrap_or_default();
    if let Some(themed) = gio_icon.downcast_ref::<gio::ThemedIcon>() {
        for name in themed.names() {
            names.push(name.to_string());
        }
    }
    for last in [
        "package-x-generic",
        "package-x-generic-symbolic",
        "text-x-generic",
        "text-x-generic-symbolic",
    ] {
        names.push(last.to_string());
    }
    let mut seen = std::collections::HashSet::new();
    names.retain(|name| seen.insert(name.clone()));
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    gio::ThemedIcon::from_names(&refs).upcast()
}

fn gio_icon_for(path: &std::path::Path) -> gio::Icon {
    let file = gio::File::for_path(path);
    if let Ok(info) = file.query_info(
        "standard::icon,standard::content-type",
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>,
    ) {
        // An empty file is sniffed as application/x-zerosize, which throws
        // away the type the filename already states.
        if info.content_type().as_deref() != Some("application/x-zerosize") {
            if let Some(icon) = info.icon() {
                return icon;
            }
        }
    }
    // A non-empty dummy byte keeps Gio from treating a missing file as
    // zerosize, so the extension (for example `.exe`) still decides.
    let (content_type, _) = gio::content_type_guess(Some(path), &[0]);
    gio::content_type_get_icon(content_type.as_str())
}

fn report_icon(path: &std::path::Path, icon: &gio::Icon) {
    let Some(themed) = icon.downcast_ref::<gio::ThemedIcon>() else {
        eprintln!("au-icon {} (no themed icon)", path.display());
        return;
    };
    let names = themed.names();
    let chain = names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>()
        .join(" -> ");
    let resolved = gdk::Display::default().and_then(|display| {
        let theme = gtk::IconTheme::for_display(&display);
        let first = names.first()?.as_str();
        let fallbacks: Vec<&str> = names.iter().skip(1).map(|name| name.as_str()).collect();
        let paintable = theme.lookup_icon(
            first,
            &fallbacks,
            28,
            1,
            gtk::TextDirection::Ltr,
            gtk::IconLookupFlags::empty(),
        );
        let named = paintable.icon_name().map(|name| name.display().to_string());
        let file = paintable
            .file()
            .and_then(|file| file.path())
            .map(|file| file.display().to_string());
        match (named, file) {
            (Some(name), Some(file)) => Some(format!("{name} ({file})")),
            (Some(name), None) => Some(name),
            (None, Some(file)) => Some(file),
            (None, None) => None,
        }
    });
    eprintln!(
        "au-icon {} chain=[{}] resolved={}",
        path.display(),
        chain,
        resolved.as_deref().unwrap_or("none")
    );
}

fn round_button(icon: &str, tip: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tip)
        .css_classes(["au-round", "circular", "flat"])
        .valign(gtk::Align::Center)
        .build();
    if let Some(image) = button
        .child()
        .and_then(|child| child.downcast::<gtk::Image>().ok())
    {
        image.set_pixel_size(10);
    }
    button
}

pub fn about_remaining(elapsed_secs: f64, fraction: f64) -> String {
    if fraction < 0.02 || !elapsed_secs.is_finite() {
        return "Estimating…".into();
    }
    let remaining = (elapsed_secs * (1.0 - fraction) / fraction)
        .round()
        .max(0.0) as u64;
    if remaining < 2 {
        "About a second".into()
    } else if remaining < 60 {
        format!("About {remaining} seconds")
    } else {
        let minutes = (remaining + 30) / 60;
        if minutes <= 1 {
            "About 1 minute".into()
        } else {
            format!("About {minutes} minutes")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::about_remaining;

    #[test]
    fn remaining_time_reads_about_n_units() {
        assert_eq!(about_remaining(0.0, 0.0), "Estimating…");
        assert_eq!(about_remaining(4.0, 0.5), "About 4 seconds");
        assert_eq!(about_remaining(30.0, 0.5), "About 30 seconds");
        assert_eq!(about_remaining(90.0, 0.5), "About 2 minutes");
    }
}
