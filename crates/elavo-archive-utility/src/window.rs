//! The Archive Utility window. Idle, it shows the options for the selected format.
//! Dragging files over it crossfades to the drop targets: the upper area
//! extracts, the lower area compresses. Letting go, or leaving the window,
//! returns to the options.

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use elavo_archive_utility_core::{
    all_archives, create_choices, parse_size, suggest_archive_path, suggest_extract_parent, Cancel,
    CompressRequest, ExtractRequest, Format, FormatOptions, Level, Outcome, Progress, Settings,
    SPLIT_PRESETS,
};

const WIDTH: i32 = 420;
const HEIGHT: i32 = 500;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Zone {
    Extract,
    Compress,
}

/// A window action's body.
type Handler = Box<dyn Fn(&Rc<Window>)>;

enum JobEvent {
    Progress(Progress),
    Done(elavo_archive_utility_core::Result<Outcome>),
}

pub struct Window {
    window: gtk::ApplicationWindow,
    settings: Rc<RefCell<Settings>>,
    busy: Cell<bool>,
    suppress: Cell<bool>,
    cancel: RefCell<Cancel>,
    stack: gtk::Stack,
    badge: gtk::DrawingArea,
    format_label: gtk::Label,
    extract_zone: gtk::Box,
    compress_zone: gtk::Box,
    method: gtk::Label,
    level: gtk::Scale,
    split: gtk::Entry,
    split_menu: gtk::MenuButton,
    password: gtk::Entry,
    aes: gtk::CheckButton,
    forks: gtk::CheckButton,
    verify: gtk::CheckButton,
    delete_after: gtk::CheckButton,
    separate: gtk::CheckButton,
    hint: gtk::Label,
    progress_title: gtk::Label,
    progress_bar: gtk::ProgressBar,
    progress_detail: gtk::Label,
    done_title: gtk::Label,
    done_detail: gtk::Label,
    show_button: gtk::Button,
    done_path: RefCell<Option<PathBuf>>,
    last_extract: RefCell<Vec<PathBuf>>,
    popover: gtk::Popover,
}

impl Window {
    pub fn new(app: &gtk::Application, _compact: bool) -> Rc<Self> {
        let settings = Rc::new(RefCell::new(Settings::load()));
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Archive Utility")
            .default_width(WIDTH)
            .default_height(HEIGHT)
            .resizable(true)
            .css_classes(["background", "au"])
            .build();

        // The theme's title bar, as in System Settings: window buttons from
        // the system layout, no title text, and the format pill at the end.
        let header = gtk::HeaderBar::builder()
            .show_title_buttons(true)
            .css_classes(["au-header"])
            .build();
        header.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));

        let badge = gtk::DrawingArea::builder()
            .content_width(26)
            .content_height(26)
            .css_classes(["au-badge"])
            .valign(gtk::Align::Center)
            .build();
        let format_label = gtk::Label::builder()
            .css_classes(["au-format-name"])
            .build();
        let pill_face = gtk::Box::builder()
            .spacing(6)
            .valign(gtk::Align::Center)
            .css_classes(["au-pill-main"])
            .build();
        format_label.set_valign(gtk::Align::Center);
        pill_face.append(&badge);
        pill_face.append(&format_label);

        let popover = gtk::Popover::builder()
            .css_classes(["au-menu"])
            .position(gtk::PositionType::Bottom)
            .has_arrow(false)
            .build();
        let list = format_list(settings.borrow().format.pill());
        popover.set_child(Some(&list));
        let chevron = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        let up = gtk::Image::from_icon_name("pan-up-symbolic");
        let down = gtk::Image::from_icon_name("pan-down-symbolic");
        up.set_pixel_size(8);
        down.set_pixel_size(8);
        up.set_valign(gtk::Align::Center);
        down.set_valign(gtk::Align::Center);
        chevron.append(&up);
        chevron.append(&down);
        let menu = gtk::MenuButton::builder()
            .css_classes(["au-pill-chevron"])
            .popover(&popover)
            .child(&chevron)
            .valign(gtk::Align::Center)
            .tooltip_text("Format")
            .build();
        let pill = gtk::Box::builder()
            .css_classes(["au-pill"])
            .valign(gtk::Align::Center)
            .build();
        pill.append(&pill_face);
        pill.append(&menu);
        header.pack_end(&pill);
        window.set_titlebar(Some(&header));

        let stack = gtk::Stack::builder()
            .vexpand(true)
            .hexpand(true)
            .vhomogeneous(true)
            .hhomogeneous(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(180)
            .build();
        let (drop_page, extract_zone, compress_zone, hint) = drop_page();
        let (
            options_page,
            method,
            level,
            split,
            split_menu,
            password,
            aes,
            forks,
            verify,
            delete_after,
            separate,
        ) = options_page();
        let (running, progress_title, progress_bar, progress_detail) = progress_page();
        let (done, done_title, done_detail, show_button) = done_page();
        options_page.append(&hint);
        stack.add_named(&options_page, Some("options"));
        stack.add_named(&drop_page, Some("drop"));
        stack.add_named(&running, Some("running"));
        stack.add_named(&done, Some("done"));
        stack.set_visible_child_name("options");
        window.set_child(Some(&stack));

        let this = Rc::new(Self {
            window,
            settings,
            busy: Cell::new(false),
            suppress: Cell::new(false),
            cancel: RefCell::new(Cancel::new()),
            stack,
            badge,
            format_label,
            extract_zone,
            compress_zone,
            method,
            level,
            split,
            split_menu,
            password,
            aes,
            forks,
            verify,
            delete_after,
            separate,
            hint,
            progress_title,
            progress_bar,
            progress_detail,
            done_title,
            done_detail,
            show_button,
            done_path: RefCell::new(None),
            last_extract: RefCell::new(Vec::new()),
            popover,
        });

        let format = this.settings.borrow().format.pill();
        this.bind_badge(format);
        this.load_controls(format);
        this.wire(&list);
        this.apply_shot();
        this
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn offer(&self, paths: Vec<PathBuf>, autostart: bool) {
        if paths.is_empty() || !autostart {
            return;
        }
        if all_archives(&paths) {
            self.begin_extract(paths);
        } else {
            self.begin_compress(paths);
        }
    }

    fn bind_badge(&self, format: Format) {
        self.badge.set_draw_func(move |_area, cr, width, height| {
            draw_badge(cr, format, width as f64, height as f64);
        });
    }

    fn wire(self: &Rc<Self>, list: &gtk::ListBox) {
        self.add_actions();
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(window) = weak.upgrade() {
                window.cancel.borrow().cancel();
            }
            glib::Propagation::Proceed
        });
        attach_window_drag(&self.window, Rc::downgrade(self));

        let weak = Rc::downgrade(self);
        list.connect_row_activated(move |_, row| {
            let Some(window) = weak.upgrade() else { return };
            let choices = create_choices();
            let Some(choice) = choices.get(row.index() as usize) else {
                return;
            };
            window.popover.popdown();
            window.select_format(choice.format);
            if !choice.available {
                window.hint.set_label(choice.hint);
            }
        });

        let weak = Rc::downgrade(self);
        let save = move || {
            if let Some(window) = weak.upgrade() {
                if !window.suppress.get() {
                    window.store_controls();
                }
            }
        };
        let save_level = save.clone();
        self.level.connect_value_changed(move |_| save_level());
        let save_split = save.clone();
        self.split.connect_changed(move |_| save_split());
        let save_pass = Rc::downgrade(self);
        self.password.connect_changed(move |_| {
            if let Some(window) = save_pass.upgrade() {
                window.refresh_aes();
                if !window.suppress.get() {
                    window.store_controls();
                }
            }
        });
        for check in [
            &self.aes,
            &self.forks,
            &self.verify,
            &self.delete_after,
            &self.separate,
        ] {
            let save_check = save.clone();
            check.connect_toggled(move |_| save_check());
        }

        let weak = Rc::downgrade(self);
        self.show_button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                if let Some(path) = window.done_path.borrow().clone() {
                    reveal_path(&window.window, &path);
                }
            }
        });

        let menu = context_menu();
        menu.set_parent(&self.window);
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gdk::BUTTON_SECONDARY);
        let menu_pop = menu.clone();
        gesture.connect_pressed(move |_, _, x, y| {
            menu_pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            menu_pop.popup();
        });
        self.window.add_controller(gesture);
    }

    fn add_actions(self: &Rc<Self>) {
        let win = &self.window;
        let add = |name: &str, handler: Handler| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    handler(&window);
                }
            });
            win.add_action(&action);
        };
        add(
            "preferences",
            Box::new(|window| {
                let settings = window.settings.clone();
                let weak = Rc::downgrade(window);
                crate::prefs::present(&window.window, settings, move || {
                    if let Some(window) = weak.upgrade() {
                        window.load_controls(window.settings.borrow().format.pill());
                    }
                });
            }),
        );
        add(
            "about",
            Box::new(|window| {
                let dialog = gtk::AboutDialog::builder()
                    .transient_for(&window.window)
                    .modal(true)
                    .destroy_with_parent(true)
                    .program_name("Elavo Archive Utility")
                    .version(env!("CARGO_PKG_VERSION"))
                    .comments("Compress files and extract archives.")
                    .copyright("© 2026 Kevin Doyle")
                    .license_type(gtk::License::MitX11)
                    .website("https://github.com/elavo-io/elavo-archive-utility")
                    .authors(["Kevin Doyle"])
                    .logo_icon_name("io.elavo.ArchiveUtility")
                    .build();
                dialog.present();
            }),
        );
        add("open-files", Box::new(|window| window.pick(false)));
        let preset = gio::SimpleAction::new("split-preset", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        preset.connect_activate(move |_, value| {
            let Some(window) = weak.upgrade() else { return };
            if let Some(text) = value.and_then(|value| value.str()) {
                window.split.set_text(text);
            }
        });
        win.add_action(&preset);
        add("open-archive", Box::new(|window| window.pick(true)));
        add("cancel", Box::new(|window| window.cancel.borrow().cancel()));
        add("close", Box::new(|window| window.window.close()));
        add("back", Box::new(|window| window.show_idle()));

        let format_action = gio::SimpleAction::new("format", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        format_action.connect_activate(move |_, value| {
            let Some(window) = weak.upgrade() else { return };
            let Some(id) = value.and_then(|value| value.str()) else {
                return;
            };
            if let Some(format) = Format::from_id(id) {
                window.select_format(format.pill());
            }
        });
        win.add_action(&format_action);

        if let Some(app) = self.window.application() {
            for choice in create_choices() {
                if let Some(accel) = choice.format.shortcut_accel() {
                    let detailed = format!("win.format::{}", choice.format.id());
                    app.set_accels_for_action(&detailed, &[accel]);
                }
            }
        }

        let progress =
            gio::SimpleAction::new("progress", Some(glib::VariantTy::new("(ds)").unwrap()));
        let weak = Rc::downgrade(self);
        progress.connect_activate(move |_, value| {
            let Some(window) = weak.upgrade() else { return };
            let Some((fraction, message)) = value.and_then(|v| v.get::<(f64, String)>()) else {
                return;
            };
            window.progress_bar.set_fraction(fraction.clamp(0.0, 1.0));
            window.progress_detail.set_label(&message);
        });
        win.add_action(&progress);

        let pulse = gio::SimpleAction::new("pulse", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        pulse.connect_activate(move |_, value| {
            let Some(window) = weak.upgrade() else { return };
            window.progress_bar.pulse();
            if let Some(message) = value.and_then(|v| v.str()) {
                window.progress_detail.set_label(message);
            }
        });
        win.add_action(&pulse);

        let finish = gio::SimpleAction::new("finish", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        finish.connect_activate(move |_, value| {
            let Some(window) = weak.upgrade() else { return };
            window.busy.set(false);
            let text = value
                .and_then(|v| v.str())
                .unwrap_or("err\nSomething went wrong.");
            let (kind, detail) = text.split_once('\n').unwrap_or(("err", text));
            if kind == "ok" {
                let path = PathBuf::from(detail);
                window.done_path.replace(Some(path.clone()));
                window.done_title.set_label("Finished");
                window.done_detail.set_label(&path.display().to_string());
                window.show_button.set_visible(true);
                window.stack.set_visible_child_name("done");
                window.password.set_text("");
                if window.settings.borrow().open_destination {
                    reveal_path(&window.window, &path);
                }
            } else if detail.starts_with("Cancelled") {
                window.hint.set_label("Cancelled.");
                window.show_idle();
            } else if detail.to_lowercase().contains("password") {
                window.hint.set_label(detail);
                window.show_idle();
                window.ask_password(detail);
            } else {
                window.done_path.replace(None);
                window.done_title.set_label("Couldn't finish");
                window.done_detail.set_label(detail);
                window.show_button.set_visible(false);
                window.stack.set_visible_child_name("done");
            }
        });
        win.add_action(&finish);
    }

    fn select_format(&self, format: Format) {
        let format = format.pill();
        self.store_controls();
        self.settings.borrow_mut().format = format;
        self.load_controls(format);
        let _ = self.settings.borrow().save();
        let choice = create_choices()
            .into_iter()
            .find(|choice| choice.format == format);
        if let Some(choice) = choice {
            if choice.available {
                self.hint.set_label("");
            } else {
                self.hint.set_label(choice.hint);
            }
        }
    }

    fn load_controls(&self, format: Format) {
        self.suppress.set(true);
        let options = self.settings.borrow().options_of(format);
        self.format_label.set_label(format.label());
        self.set_method_label(format, options.level);
        self.bind_badge(format);
        self.badge.queue_draw();
        self.refresh_checks();
        self.level.set_value(options.level.index());
        self.level.set_sensitive(format.supports_level());
        self.split.set_text(&options.split);
        let can_split = format.supports_split();
        self.split.set_sensitive(can_split);
        self.split_menu.set_sensitive(can_split);
        self.refresh_password(format);
        self.forks.set_active(options.exclude_macos);
        self.verify.set_active(options.verify);
        self.delete_after.set_active(options.delete_after);
        self.separate.set_active(options.separate);
        self.aes.set_active(options.aes256);
        self.refresh_aes();
        self.suppress.set(false);
    }

    fn store_controls(&self) {
        if self.suppress.get() {
            return;
        }
        let format = self.settings.borrow().format.pill();
        let level = Level::from_index(self.level.value());
        self.set_method_label(format, level);
        let options = FormatOptions {
            level,
            split: self.split.text().to_string(),
            aes256: self.aes.is_active(),
            exclude_macos: self.forks.is_active(),
            verify: self.verify.is_active(),
            delete_after: self.delete_after.is_active(),
            separate: self.separate.is_active(),
        };
        self.settings.borrow_mut().set_options(format, options);
        let _ = self.settings.borrow().save();
    }

    fn set_method_label(&self, format: Format, level: Level) {
        self.method.set_label(format.method_text(level));
    }

    /// Only 7Z and ZIP take a password. For the others the field is off and
    /// says why, so a password is never silently ignored.
    fn refresh_password(&self, format: Format) {
        let can = format.supports_password();
        if !can {
            self.password.set_text("");
        }
        self.password.set_sensitive(can);
        if can {
            self.password.set_placeholder_text(Some("Password"));
            self.password.set_tooltip_text(None);
        } else {
            self.password
                .set_placeholder_text(Some(&format!("{} has no password", format.label())));
            self.password
                .set_tooltip_text(Some("Only 7Z and ZIP archives can be encrypted."));
        }
    }

    fn current_options(&self) -> FormatOptions {
        self.store_controls();
        let settings = self.settings.borrow();
        settings.options_of(settings.format.pill())
    }

    fn refresh_checks(&self) {
        let selected = self.settings.borrow().format.pill();
        let Some(list) = self
            .popover
            .child()
            .and_then(|child| child.downcast::<gtk::ListBox>().ok())
        else {
            return;
        };
        let choices = create_choices();
        let mut index = 0;
        let mut child = list.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Some(row) = widget.downcast_ref::<gtk::ListBoxRow>() {
                let on = choices
                    .get(index)
                    .is_some_and(|choice| choice.format == selected);
                if let Some(contents) = row.child() {
                    set_check(&contents, on);
                }
            }
            index += 1;
            child = next;
        }
    }

    fn refresh_aes(&self) {
        let zip = self.settings.borrow().format.pill() == Format::Zip;
        let has_password = !self.password.text().is_empty();
        self.aes.set_sensitive(zip && has_password);
    }

    fn begin_compress(&self, paths: Vec<PathBuf>) {
        if self.busy.get() || paths.is_empty() {
            return;
        }
        let settings = self.settings.borrow().clone();
        let format = settings.format.pill().for_inputs(&paths);
        if !format.create_ready(elavo_archive_utility_core::tools()) {
            self.hint.set_label(format.pill().create_hint());
            self.show_idle();
            return;
        }
        let options = self.current_options();
        let split = if format.supports_split() {
            match parse_size(&options.split) {
                Some(bytes) => bytes,
                None => {
                    self.hint
                        .set_label("Couldn't read the split size. Example: 5 MB");
                    return;
                }
            }
        } else {
            None
        };
        let password_text = self.password.text().to_string();
        let password = if password_text.is_empty() {
            None
        } else {
            Some(password_text)
        };
        if password.is_some() && !format.supports_password() {
            self.hint
                .set_label(&format!("{} does not take a password.", format.label()));
            return;
        }
        let output = suggest_archive_path(&paths, format, &settings.output);
        let req = CompressRequest {
            inputs: paths,
            output: output.clone(),
            format,
            level: options.level,
            password,
            encrypt_names: settings.encrypt_names && format.supports_header_encryption(),
            aes256: options.aes256 && format.supports_aes(),
            split,
            exclude_macos: options.exclude_macos,
            verify: options.verify,
            delete_after: options.delete_after,
            separate: options.separate,
        };
        self.progress_title
            .set_label(&format!("Compressing {}", file_name(&output)));
        self.launch(move |cancel, progress| {
            elavo_archive_utility_core::compress(&req, cancel, progress)
        });
    }

    fn begin_extract(&self, paths: Vec<PathBuf>) {
        let password = self.password.text().to_string();
        let password = if password.is_empty() {
            None
        } else {
            Some(password)
        };
        self.last_extract.replace(paths.clone());
        self.extract_with(paths, password);
    }

    fn extract_with(&self, paths: Vec<PathBuf>, password: Option<String>) {
        if self.busy.get() || paths.is_empty() {
            return;
        }
        let settings = self.settings.borrow().clone();
        let destination = suggest_extract_parent(&paths[0], &settings.output);
        let reqs: Vec<ExtractRequest> = paths
            .iter()
            .map(|archive| ExtractRequest {
                archive: archive.clone(),
                destination: destination.clone(),
                password: password.clone(),
                new_folder: settings.new_folder,
                conflict: settings.conflict,
            })
            .collect();
        self.progress_title.set_label("Extracting");
        self.launch(move |cancel, mut progress| {
            let mut last = None;
            for (index, req) in reqs.iter().enumerate() {
                cancel.check()?;
                let name = file_name(&req.archive);
                let total = reqs.len() as f64;
                let outcome = elavo_archive_utility_core::extract(req, cancel, |update| {
                    let fraction = update
                        .fraction
                        .map(|fraction| (index as f64 + fraction) / total);
                    progress(Progress {
                        fraction,
                        message: format!("{name}  {}", update.message),
                    });
                })?;
                last = Some(outcome);
            }
            last.ok_or(elavo_archive_utility_core::Error::Empty)
        });
    }

    fn ask_password(&self, message: &str) {
        let prompt = crate::dialog::password(&self.window, message);
        let dialog = prompt.window.clone();
        let entry = prompt.entry.clone();
        let open = prompt.open.clone();
        let owner = self.window.clone();
        let paths = self.last_extract.borrow().clone();
        let dialog_ok = dialog.clone();
        let entry_ok = entry.clone();
        open.connect_clicked(move |_| {
            let password = entry_ok.text().to_string();
            let _ = gtk::prelude::WidgetExt::activate_action(
                &owner,
                "win.extract-now",
                Some(&format!("{password}\n{}", paths_join(&paths)).to_variant()),
            );
            dialog_ok.close();
        });
        dialog.present();
        entry.grab_focus();
    }

    fn launch<F>(&self, work: F)
    where
        F: FnOnce(
                &Cancel,
                Box<dyn FnMut(Progress) + Send>,
            ) -> elavo_archive_utility_core::Result<Outcome>
            + Send
            + 'static,
    {
        self.busy.set(true);
        self.hint.set_label("");
        self.progress_bar.set_fraction(0.0);
        self.progress_detail.set_label("Starting…");
        self.stack.set_visible_child_name("running");
        let cancel = Cancel::new();
        *self.cancel.borrow_mut() = cancel.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let tx_progress = tx.clone();
            let result = work(
                &cancel,
                Box::new(move |progress| {
                    let _ = tx_progress.send(JobEvent::Progress(progress));
                }),
            );
            let _ = tx.send(JobEvent::Done(result));
        });
        let rx = Rc::new(RefCell::new(rx));
        let window = self.window.clone();
        // Keep the process alive until the worker has returned, even if the
        // window is closed, so a cancelled job removes its temporary files.
        let mut hold = self.window.application().map(|app| app.hold());
        glib::timeout_add_local(Duration::from_millis(40), move || {
            let mut finished = None;
            loop {
                match rx.borrow().try_recv() {
                    Ok(JobEvent::Progress(progress)) => {
                        if let Some(fraction) = progress.fraction {
                            let _ = gtk::prelude::WidgetExt::activate_action(
                                &window,
                                "win.progress",
                                Some(&(fraction, progress.message).to_variant()),
                            );
                        } else {
                            let _ = gtk::prelude::WidgetExt::activate_action(
                                &window,
                                "win.pulse",
                                Some(&progress.message.to_variant()),
                            );
                        }
                    }
                    Ok(JobEvent::Done(result)) => finished = Some(result),
                    Err(_) => break,
                }
            }
            if let Some(result) = finished {
                let text = match &result {
                    Ok(outcome) => format!("ok\n{}", outcome.path.display()),
                    Err(err) => format!("err\n{err}"),
                };
                let _ = gtk::prelude::WidgetExt::activate_action(
                    &window,
                    "win.finish",
                    Some(&text.to_variant()),
                );
                hold.take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn pick(&self, archive: bool) {
        let dialog = gtk::FileDialog::new();
        dialog.set_title(if archive { "Archive" } else { "Files" });
        dialog.set_modal(true);
        let window = self.window.clone();
        if archive {
            dialog.open(
                Some(&self.window),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            let _ = gtk::prelude::WidgetExt::activate_action(
                                &window,
                                "win.extract-paths",
                                Some(&path.display().to_string().to_variant()),
                            );
                        }
                    }
                },
            );
        } else {
            dialog.open_multiple(
                Some(&self.window),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(model) = result {
                        let mut paths = Vec::new();
                        for index in 0..model.n_items() {
                            if let Some(file) = model
                                .item(index)
                                .and_then(|item| item.downcast::<gio::File>().ok())
                            {
                                if let Some(path) = file.path() {
                                    paths.push(path.display().to_string());
                                }
                            }
                        }
                        if !paths.is_empty() {
                            let _ = gtk::prelude::WidgetExt::activate_action(
                                &window,
                                "win.compress-paths",
                                Some(&paths.join("\n").to_variant()),
                            );
                        }
                    }
                },
            );
        }
    }

    fn apply_shot(&self) {
        let Ok(mode) = std::env::var("ELAVO_ARCHIVE_UTILITY_SCREENSHOT") else {
            return;
        };
        // A fixed format makes the pictures comparable.
        if let Some(format) = std::env::var("ELAVO_ARCHIVE_UTILITY_SHOT_FORMAT")
            .ok()
            .and_then(|id| Format::from_id(&id))
        {
            self.select_format(format);
        }
        let takes_password = self.settings.borrow().format.supports_password();
        if !mode.starts_with("drag") && takes_password {
            self.suppress.set(true);
            self.password.set_text("secret");
            let zip = self.settings.borrow().format.pill() == Format::Zip;
            self.refresh_aes();
            if zip {
                self.aes.set_active(true);
            }
            self.suppress.set(false);
        }
        if let Ok(index) = std::env::var("ELAVO_ARCHIVE_UTILITY_SHOT_LEVEL") {
            if let Ok(index) = index.parse::<f64>() {
                self.level.set_value(index);
            }
        }
        match mode.as_str() {
            "drag" | "drag-extract" => {
                self.stack.set_visible_child_name("drop");
                self.extract_zone.add_css_class("over");
            }
            "drag-compress" => {
                self.stack.set_visible_child_name("drop");
                self.compress_zone.add_css_class("over");
            }
            "formats" | "menu" => {
                let pop = self.popover.clone();
                glib::idle_add_local(move || {
                    pop.popup();
                    glib::ControlFlow::Break
                });
            }
            "presets" | "split" => {
                let pop = self.split_menu.clone();
                // Leave time for a screenshot harness to place the window
                // before the menu measures its position.
                glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
                    pop.popup();
                    glib::ControlFlow::Break
                });
            }
            "prefs" | "about" => {
                let window = self.window.clone();
                let action = if mode == "prefs" {
                    "win.preferences"
                } else {
                    "win.about"
                };
                glib::timeout_add_local(std::time::Duration::from_millis(800), move || {
                    let _ = gtk::prelude::WidgetExt::activate_action(&window, action, None);
                    glib::ControlFlow::Break
                });
            }
            "password" => {
                let window = self.window.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(800), move || {
                    let prompt = crate::dialog::password(
                        &window,
                        "\u{201c}photos.7z\u{201d} is encrypted. Enter its password to open it.",
                    );
                    prompt.window.present();
                    prompt.entry.grab_focus();
                    glib::ControlFlow::Break
                });
            }
            _ => {}
        }
    }

    fn show_idle(&self) {
        if self.busy.get() {
            return;
        }
        let page = self
            .stack
            .visible_child_name()
            .map(|name| name.to_string())
            .unwrap_or_default();
        if page == "running" || page == "done" {
            return;
        }
        self.extract_zone.remove_css_class("over");
        self.compress_zone.remove_css_class("over");
        self.stack.set_visible_child_name("options");
    }

    fn show_drag(&self, y: f64) {
        if self.busy.get() {
            return;
        }
        self.stack.set_visible_child_name("drop");
        self.highlight(self.zone_at(y));
    }

    fn highlight(&self, zone: Zone) {
        self.extract_zone.remove_css_class("over");
        self.compress_zone.remove_css_class("over");
        match zone {
            Zone::Extract => self.extract_zone.add_css_class("over"),
            Zone::Compress => self.compress_zone.add_css_class("over"),
        }
    }

    /// The lower band is the compress target. Everything above it extracts.
    fn zone_at(&self, y: f64) -> Zone {
        let height = self.window.height().max(1) as f64;
        if y >= height - 96.0 {
            Zone::Compress
        } else {
            Zone::Extract
        }
    }

    fn dropped(&self, paths: Vec<PathBuf>, zone: Zone) {
        self.hint.set_label("");
        match zone {
            Zone::Compress => self.begin_compress(paths),
            // Files are extracted; one that is not a known archive is
            // handed to 7-Zip, which reports it if it cannot open it.
            // Folders cannot be extracted, so a drop of only folders
            // compresses them instead.
            Zone::Extract => {
                let (folders, files): (Vec<PathBuf>, Vec<PathBuf>) =
                    paths.into_iter().partition(|path| path.is_dir());
                if files.is_empty() {
                    self.begin_compress(folders);
                } else {
                    self.begin_extract(files);
                    if !folders.is_empty() {
                        self.hint.set_label(
                            "Folders can't be extracted. Drop them on the lower area to compress them.",
                        );
                    }
                }
            }
        }
    }
}

pub fn wire_extra(window: &Rc<Window>) {
    let compress = gio::SimpleAction::new("compress-paths", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(window);
    compress.connect_activate(move |_, value| {
        let Some(window) = weak.upgrade() else { return };
        let Some(text) = value.and_then(|v| v.str()) else {
            return;
        };
        let paths = text
            .lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect();
        window.begin_compress(paths);
    });
    window.window.add_action(&compress);

    let extract = gio::SimpleAction::new("extract-paths", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(window);
    extract.connect_activate(move |_, value| {
        let Some(window) = weak.upgrade() else { return };
        let Some(text) = value.and_then(|v| v.str()) else {
            return;
        };
        let paths = text
            .lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect();
        window.begin_extract(paths);
    });
    window.window.add_action(&extract);

    let retry = gio::SimpleAction::new("extract-now", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(window);
    retry.connect_activate(move |_, value| {
        let Some(window) = weak.upgrade() else { return };
        let Some(text) = value.and_then(|v| v.str()) else {
            return;
        };
        let Some((password, rest)) = text.split_once('\n') else {
            return;
        };
        let paths = rest
            .lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect();
        let password = if password.is_empty() {
            None
        } else {
            Some(password.to_string())
        };
        window.extract_with(paths, password);
    });
    window.window.add_action(&retry);
}

fn paths_join(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_list(selected: Format) -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    for choice in create_choices() {
        let row = gtk::Box::builder().spacing(8).build();
        let badge = gtk::DrawingArea::builder()
            .content_width(22)
            .content_height(22)
            .build();
        let format = choice.format;
        badge.set_draw_func(move |_, cr, w, h| draw_badge(cr, format, w as f64, h as f64));
        let name = gtk::Label::builder()
            .label(choice.format.label())
            .xalign(0.0)
            .hexpand(true)
            .build();
        let check = gtk::Label::builder()
            .label("✓")
            .css_classes(["au-check"])
            .visible(false)
            .build();
        let accel = gtk::Label::builder()
            .label(choice.format.shortcut_label().unwrap_or(""))
            .css_classes(["au-accel"])
            .build();
        row.append(&badge);
        row.append(&name);
        row.append(&check);
        row.append(&accel);
        list.append(&row);
        if let Some(list_row) = list
            .last_child()
            .and_then(|widget| widget.downcast::<gtk::ListBoxRow>().ok())
        {
            if !choice.available {
                list_row.add_css_class("au-missing");
                list_row.set_tooltip_text(Some(choice.hint));
            }
            check.set_visible(choice.format == selected);
        }
    }
    // Mark the selected format. The list is rebuilt only once, so the check
    // is updated whenever the popover is shown.
    list
}

fn drop_page() -> (gtk::Box, gtk::Box, gtk::Box, gtk::Label) {
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["au-drop"])
        .build();
    let extract = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .css_classes(["au-zone", "au-extract"])
        .spacing(8)
        .build();
    let art = gtk::DrawingArea::builder()
        .content_width(150)
        .content_height(110)
        .halign(gtk::Align::Center)
        .build();
    art.set_draw_func(|area, cr, w, h| draw_stack(cr, &area.color(), w as f64, h as f64));
    let title = gtk::Label::builder()
        .label("Drop here to extract")
        .css_classes(["au-extract-title"])
        .halign(gtk::Align::Center)
        .build();
    extract.append(&art);
    extract.append(&title);
    let hint = gtk::Label::builder()
        .css_classes(["au-hint"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .halign(gtk::Align::Center)
        .margin_start(16)
        .margin_end(16)
        .build();
    let compress = gtk::Box::builder()
        .css_classes(["au-zone", "au-compress"])
        .height_request(64)
        .build();
    let compress_label = gtk::Label::builder()
        .label("Drop here to compress")
        .css_classes(["au-compress-title"])
        .halign(gtk::Align::Center)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    compress.append(&compress_label);
    page.append(&extract);
    page.append(&compress);
    (page, extract, compress, hint)
}

fn options_page() -> (
    gtk::Box,
    gtk::Label,
    gtk::Scale,
    gtk::Entry,
    gtk::MenuButton,
    gtk::Entry,
    gtk::CheckButton,
    gtk::CheckButton,
    gtk::CheckButton,
    gtk::CheckButton,
    gtk::CheckButton,
) {
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .css_classes(["au-options"])
        .build();
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["au-card"])
        .build();
    card.set_overflow(gtk::Overflow::Visible);
    let method = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["au-method"])
        .label(Level::Normal.description())
        .build();
    let level = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 5.0, 1.0);
    level.set_draw_value(false);
    level.set_round_digits(0);
    level.set_increments(1.0, 1.0);
    level.add_css_class("au-level");
    for index in 0..6 {
        level.add_mark(index as f64, gtk::PositionType::Bottom, None);
    }
    level.connect_value_changed(|scale| {
        let snapped = scale.value().round().clamp(0.0, 5.0);
        if (scale.value() - snapped).abs() > 0.01 {
            scale.set_value(snapped);
        }
    });
    // The two ends in the same plain words as the line above the slider.
    let ticks = gtk::Box::builder().css_classes(["au-ticks"]).build();
    for (level, align) in [
        (Level::Store, gtk::Align::Start),
        (Level::Slowest, gtk::Align::End),
    ] {
        ticks.append(
            &gtk::Label::builder()
                .label(level.description())
                .halign(align)
                .hexpand(true)
                .css_classes(["dim-label", "au-small"])
                .build(),
        );
    }
    let split_row = labeled("Split:");
    let split = gtk::Entry::builder()
        .placeholder_text("Example: 5 MB")
        .hexpand(true)
        .build();
    // A plain menu from the theme. A list box in a popover selected and
    // focused rows as soon as it opened.
    let presets = gio::Menu::new();
    for preset in SPLIT_PRESETS {
        let item = gio::MenuItem::new(Some(preset.label), None);
        item.set_action_and_target_value(
            Some("win.split-preset"),
            Some(&preset.label.to_variant()),
        );
        presets.append_item(&item);
    }
    let split_menu = gtk::MenuButton::builder()
        .icon_name("pan-down-symbolic")
        .css_classes(["au-split-menu"])
        .menu_model(&presets)
        .tooltip_text("Split presets")
        .valign(gtk::Align::Center)
        .build();
    split_row.append(&split);
    split_row.append(&split_menu);
    let password_row = labeled("Password:");
    let password = gtk::Entry::builder()
        .placeholder_text("Password")
        .visibility(false)
        .hexpand(true)
        .build();
    if let Ok(bytes) = gio::resources_lookup_data(
        "/io/elavo/ArchiveUtility/icons/au-lock.png",
        gio::ResourceLookupFlags::NONE,
    ) {
        if let Ok(texture) = gdk::Texture::from_bytes(&bytes) {
            password.set_icon_from_paintable(gtk::EntryIconPosition::Primary, Some(&texture));
        }
    }
    password.set_icon_from_icon_name(
        gtk::EntryIconPosition::Secondary,
        Some("view-reveal-symbolic"),
    );
    password.connect_icon_release(|entry, position| {
        if position == gtk::EntryIconPosition::Secondary {
            let show = !entry.property::<bool>("visibility");
            entry.set_visibility(show);
            entry.set_icon_from_icon_name(
                gtk::EntryIconPosition::Secondary,
                Some(if show {
                    "view-conceal-symbolic"
                } else {
                    "view-reveal-symbolic"
                }),
            );
        }
    });
    password_row.append(&password);
    let aes = gtk::CheckButton::builder()
        .label("Use AES-256 encryption")
        .build();
    card.append(&method);
    card.append(&level);
    card.append(&ticks);
    card.append(&split_row);
    card.append(&password_row);
    card.append(&aes);
    let forks = gtk::CheckButton::with_label("Exclude Mac resource forks");
    let verify = gtk::CheckButton::with_label("Verify compression integrity");
    let delete_after = gtk::CheckButton::with_label("Delete file(s) after compression");
    let separate = gtk::CheckButton::with_label("Archive items separately");
    page.append(&card);
    page.append(&forks);
    page.append(&verify);
    page.append(&delete_after);
    page.append(&separate);
    (
        page,
        method,
        level,
        split,
        split_menu,
        password,
        aes,
        forks,
        verify,
        delete_after,
        separate,
    )
}

fn labeled(text: &str) -> gtk::Box {
    let row = gtk::Box::builder().spacing(8).build();
    row.append(
        &gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .width_chars(10)
            .build(),
    );
    row
}

fn progress_page() -> (gtk::Box, gtk::Label, gtk::ProgressBar, gtk::Label) {
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .spacing(10)
        .css_classes(["au-center"])
        .build();
    let title = gtk::Label::builder()
        .css_classes(["au-extract-title"])
        .label("Working")
        .build();
    let bar = gtk::ProgressBar::builder()
        .show_text(false)
        .hexpand(true)
        .build();
    let detail = gtk::Label::builder()
        .css_classes(["dim-label"])
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .label("Starting…")
        .build();
    let cancel = gtk::Button::builder()
        .label("Cancel")
        .action_name("win.cancel")
        .css_classes(["flat"])
        .halign(gtk::Align::Center)
        .build();
    page.append(&title);
    page.append(&bar);
    page.append(&detail);
    page.append(&cancel);
    (page, title, bar, detail)
}

fn done_page() -> (gtk::Box, gtk::Label, gtk::Label, gtk::Button) {
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .spacing(10)
        .css_classes(["au-center"])
        .build();
    let title = gtk::Label::builder()
        .css_classes(["au-extract-title"])
        .label("Finished")
        .build();
    let detail = gtk::Label::builder()
        .css_classes(["dim-label"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    let row = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::Center)
        .build();
    let show = gtk::Button::builder()
        .label("Show")
        .css_classes(["suggested-action"])
        .build();
    let back = gtk::Button::builder()
        .label("Done")
        .action_name("win.back")
        .css_classes(["flat"])
        .build();
    row.append(&show);
    row.append(&back);
    page.append(&title);
    page.append(&detail);
    page.append(&row);
    (page, title, detail, show)
}

fn context_menu() -> gtk::PopoverMenu {
    let menu = gio::Menu::new();
    menu.append(Some("Compress Files…"), Some("win.open-files"));
    menu.append(Some("Extract Archive…"), Some("win.open-archive"));
    menu.append(Some("Preferences"), Some("win.preferences"));
    menu.append(Some("About Archive Utility"), Some("win.about"));
    gtk::PopoverMenu::from_model(Some(&menu))
}

fn attach_window_drag(window: &gtk::ApplicationWindow, weak: std::rc::Weak<Window>) {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let enter = weak.clone();
    target.connect_enter(move |_, _, y| {
        if let Some(window) = enter.upgrade() {
            window.show_drag(y);
        }
        gdk::DragAction::COPY
    });
    let motion = weak.clone();
    target.connect_motion(move |_, _, y| {
        if let Some(window) = motion.upgrade() {
            window.show_drag(y);
        }
        gdk::DragAction::COPY
    });
    let leave = weak.clone();
    target.connect_leave(move |_| {
        if let Some(window) = leave.upgrade() {
            window.show_idle();
        }
    });
    target.connect_drop(move |_, value, _, y| {
        let Ok(list) = value.get::<gdk::FileList>() else {
            return false;
        };
        let paths: Vec<PathBuf> = list.files().iter().filter_map(|file| file.path()).collect();
        if paths.is_empty() {
            return false;
        }
        if let Some(window) = weak.upgrade() {
            let zone = window.zone_at(y);
            window.extract_zone.remove_css_class("over");
            window.compress_zone.remove_css_class("over");
            window.dropped(paths, zone);
        }
        true
    });
    window.add_controller(target);
}

fn draw_badge(cr: &gtk::cairo::Context, format: Format, width: f64, height: f64) {
    let (r, g, b) = format.badge_rgb();
    let radius = 5.0;
    cr.set_source_rgb(r, g, b);
    rounded(cr, 1.0, 1.0, width - 2.0, height - 2.0, radius);
    let _ = cr.fill();
    // A folded corner, so the mark reads as a file rather than a plain tile.
    let fold = (width * 0.32).min(10.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
    cr.move_to(width - 1.0 - fold, 1.0);
    cr.line_to(width - 1.0, 1.0 + fold);
    cr.line_to(width - 1.0 - fold, 1.0 + fold);
    cr.close_path();
    let _ = cr.fill();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.select_font_face(
        "Roboto",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Bold,
    );
    let size = if format.badge().chars().count() > 3 {
        6.5
    } else {
        8.0
    };
    cr.set_font_size(size);
    let text = format.badge();
    if let Ok(extents) = cr.text_extents(text) {
        cr.move_to(
            (width - extents.width()) / 2.0 - extents.x_bearing(),
            (height + extents.height()) / 2.0,
        );
        let _ = cr.show_text(text);
    }
}

/// Three sheets in the theme's text color at rising strength, so the art
/// follows light and dark.
fn draw_stack(cr: &gtk::cairo::Context, fg: &gdk::RGBA, width: f64, height: f64) {
    let (r, g, b) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
    for (x, y, alpha) in [(18.0, 8.0, 0.08), (10.0, 22.0, 0.14), (2.0, 36.0, 0.22)] {
        cr.set_source_rgba(r, g, b, alpha);
        rounded(cr, x, y, width - 36.0, height - 48.0, 8.0);
        let _ = cr.fill();
        cr.set_source_rgba(r, g, b, 0.10);
        cr.set_line_width(1.0);
        rounded(cr, x, y, width - 36.0, height - 48.0, 8.0);
        let _ = cr.stroke();
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    cr.close_path();
}

fn set_check(widget: &gtk::Widget, on: bool) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        if label.has_css_class("au-check") {
            label.set_visible(on);
        }
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        set_check(&widget, on);
        child = next;
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn reveal_path(window: &impl IsA<gtk::Window>, path: &std::path::Path) {
    let target = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf())
    };
    let uri = gio::File::for_path(target).uri();
    let display = gtk::prelude::WidgetExt::display(window.as_ref());
    let context = display.app_launch_context();
    let _ = gio::AppInfo::launch_default_for_uri(&uri, Some(&context));
}
