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
    SPLIT_PRESETS,
    CompressRequest, ExtractRequest, Format, FormatOptions, Level, Outcome, Progress, Settings,
};

const WIDTH: i32 = 420;
const HEIGHT: i32 = 500;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Zone {
    Extract,
    Compress,
}

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
    split_popover: gtk::Popover,
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
            .default_width(WIDTH + 36)
            .default_height(HEIGHT + 36)
            .resizable(true)
            .css_classes(["au"])
            .build();
        flatten_without_compositor(&window);

        let header = gtk::Box::builder()
            .spacing(8)
            .css_classes(["au-header"])
            .build();
        let controls = gtk::WindowControls::new(gtk::PackType::Start);
        controls.set_valign(gtk::Align::Center);
        header.append(&controls);

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
        flatten_without_compositor(&popover);
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
        let spacer = gtk::Box::builder().hexpand(true).build();
        header.append(&spacer);
        header.append(&pill);
        // Keep the traffic lights in the client frame. A separate title bar
        // was taller than the buttons and its square fill fought the corners.
        let titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        titlebar.set_height_request(0);
        window.set_titlebar(Some(&titlebar));

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
            split_popover,
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
        let frame = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["au-frame"])
            .build();
        frame.append(&header);
        frame.append(&stack);
        window.set_child(Some(&frame));

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
            split_popover,
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
        let add = |name: &str, handler: Box<dyn Fn(&Rc<Self>)>| {
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
        add("open-archive", Box::new(|window| window.pick(true)));
        add("cancel", Box::new(|window| window.cancel.borrow().cancel()));
        add("close", Box::new(|window| window.window.close()));
        add(
            "back",
            Box::new(|window| window.show_idle()),
        );

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

        let progress = gio::SimpleAction::new("progress", Some(glib::VariantTy::new("(ds)").unwrap()));
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
            let text = value.and_then(|v| v.str()).unwrap_or("err\nSomething went wrong.");
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
        self.method
            .set_label(&format!("Method: {}", format.method_text(level)));
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
                let on = choices.get(index).is_some_and(|choice| choice.format == selected);
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
        self.launch(move |cancel, progress| elavo_archive_utility_core::compress(&req, cancel, progress));
    }

    fn begin_extract(&self, paths: Vec<PathBuf>) {
        let password = self.password.text().to_string();
        let password = if password.is_empty() { None } else { Some(password) };
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
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .title("Password")
            .default_width(280)
            .css_classes(["au-prefs"])
            .build();
        let box_ = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();
        let label = gtk::Label::builder()
            .label(message)
            .wrap(true)
            .xalign(0.0)
            .build();
        let entry = gtk::PasswordEntry::builder()
            .show_peek_icon(true)
            .placeholder_text("Password")
            .build();
        let open = gtk::Button::builder()
            .label("Open")
            .css_classes(["au-primary", "suggested-action"])
            .halign(gtk::Align::End)
            .build();
        box_.append(&label);
        box_.append(&entry);
        box_.append(&open);
        dialog.set_child(Some(&box_));
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
        F: FnOnce(&Cancel, Box<dyn FnMut(Progress) + Send>) -> elavo_archive_utility_core::Result<Outcome>
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
            dialog.open(Some(&self.window), None::<&gio::Cancellable>, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let _ = gtk::prelude::WidgetExt::activate_action(
                            &window,
                            "win.extract-paths",
                            Some(&path.display().to_string().to_variant()),
                        );
                    }
                }
            });
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
        if !mode.starts_with("drag") {
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
                let pop = self.split_popover.clone();
                // Leave time for a screenshot harness to place the window
                // before the menu measures its position.
                glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
                    pop.popup();
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
            Zone::Extract => {
                if paths.iter().any(|path| elavo_archive_utility_core::detect(path).is_none())
                    && !paths.iter().all(|path| elavo_archive_utility_core::detect(path).is_some())
                    && paths.len() == 1
                    && elavo_archive_utility_core::detect(&paths[0]).is_none()
                {
                    // Unrecognised files are still offered to 7-Zip.
                    self.begin_extract(paths);
                } else if all_archives(&paths) || paths.iter().any(|path| elavo_archive_utility_core::detect(path).is_some())
                {
                    self.begin_extract(paths);
                } else {
                    self.begin_compress(paths);
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
        let Some(text) = value.and_then(|v| v.str()) else { return };
        let paths = text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect();
        window.begin_compress(paths);
    });
    window.window.add_action(&compress);

    let extract = gio::SimpleAction::new("extract-paths", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(window);
    extract.connect_activate(move |_, value| {
        let Some(window) = weak.upgrade() else { return };
        let Some(text) = value.and_then(|v| v.str()) else { return };
        let paths = text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect();
        window.begin_extract(paths);
    });
    window.window.add_action(&extract);

    let retry = gio::SimpleAction::new("extract-now", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(window);
    retry.connect_activate(move |_, value| {
        let Some(window) = weak.upgrade() else { return };
        let Some(text) = value.and_then(|v| v.str()) else { return };
        let Some((password, rest)) = text.split_once('\n') else { return };
        let paths = rest.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect();
        let password = if password.is_empty() { None } else { Some(password.to_string()) };
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
    art.set_draw_func(|_, cr, w, h| draw_stack(cr, w as f64, h as f64));
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
    gtk::Popover,
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
        .label("Method: Normal")
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
    // GTK clamps a mark's label into the widget, which pulls Store and Slow
    // off their ticks. These labels are positioned on the tick centers instead.
    // An overlay's size comes from the spacer, so the labels can sit on the
    // tick centers (including a small negative margin) without widening the window.
    let ticks = gtk::Overlay::new();
    ticks.set_hexpand(true);
    ticks.add_css_class("au-ticks");
    ticks.set_overflow(gtk::Overflow::Visible);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer.set_height_request(18);
    ticks.set_child(Some(&spacer));
    let tick_labels: Vec<gtk::Label> = ["Store", "Fast", "Normal", "Slow"]
        .iter()
        .map(|name| {
            let label = gtk::Label::builder()
                .label(*name)
                .halign(gtk::Align::Start)
                .valign(gtk::Align::Start)
                .css_classes(["au-dim"])
                .build();
            ticks.add_overlay(&label);
            label
        })
        .collect();
    let labels_for_layout = tick_labels.clone();
    let placed = Rc::new(RefCell::new(Vec::<i32>::new()));
    level.add_tick_callback(move |scale, _| {
        let centers = mark_centers(scale);
        if centers.len() == Level::STOPS.len() {
            let key: Vec<i32> = centers
                .iter()
                .map(|center| center.round() as i32)
                .chain(labels_for_layout.iter().map(label_text_width))
                .collect();
            let changed = key != *placed.borrow();
            if changed && place_tick_labels(&centers, &labels_for_layout) {
                *placed.borrow_mut() = key;
            }
        }
        glib::ControlFlow::Continue
    });
    let split_row = labeled("Split:");
    let split = gtk::Entry::builder()
        .placeholder_text("Example: 5 MB")
        .hexpand(true)
        .build();
    let split_popover = gtk::Popover::builder()
        .css_classes(["au-menu"])
        .position(gtk::PositionType::Bottom)
        .has_arrow(false)
        .build();
    flatten_without_compositor(&split_popover);
    let preset_list = gtk::ListBox::new();
    preset_list.add_css_class("au-split-list");
    for preset in SPLIT_PRESETS {
        preset_list.append(
            &gtk::Label::builder()
                .label(preset.label)
                .xalign(0.0)
                .build(),
        );
    }
    let split_for_preset = split.clone();
    let popover_for_preset = split_popover.clone();
    preset_list.connect_row_activated(move |_, row| {
        if let Some(preset) = SPLIT_PRESETS.get(row.index() as usize) {
            split_for_preset.set_text(preset.label);
        }
        popover_for_preset.popdown();
    });
    split_popover.set_child(Some(&preset_list));
    let split_menu = gtk::MenuButton::builder()
        .icon_name("pan-down-symbolic")
        .css_classes(["au-split-menu"])
        .popover(&split_popover)
        .tooltip_text("Split presets")
        .always_show_arrow(false)
        .valign(gtk::Align::Center)
        .build();
    let split_for_width = split.clone();
    split_popover.connect_show(move |popover| {
        popover.set_width_request(split_for_width.width().max(220));
    });
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
        split_popover,
        password,
        aes,
        forks,
        verify,
        delete_after,
        separate,
    )
}

fn flatten_without_compositor(widget: &impl IsA<gtk::Widget>) {
    widget.connect_realize(|widget| {
        if !widget.display().is_composited() {
            widget.add_css_class("au-flat");
        }
    });
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
        .css_classes(["au-progress"])
        .show_text(false)
        .hexpand(true)
        .build();
    let detail = gtk::Label::builder()
        .css_classes(["au-dim"])
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .label("Starting…")
        .build();
    let cancel = gtk::Button::builder()
        .label("Cancel")
        .action_name("win.cancel")
        .css_classes(["au-quiet"])
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
        .css_classes(["au-dim"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    let row = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).build();
    let show = gtk::Button::builder()
        .label("Show")
        .css_classes(["au-primary"])
        .build();
    let back = gtk::Button::builder()
        .label("Done")
        .action_name("win.back")
        .css_classes(["au-quiet"])
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

/// Horizontal position of a label so its center sits on an evenly spaced tick.
/// GTK draws the six marks across the trough, from its left edge to its right
/// edge, so stop `index` is at `trough_x + trough_width * index / 5`.
#[cfg(test)]
fn tick_label_x(trough_x: f64, trough_width: f64, index: f64, label_width: f64) -> f64 {
    let tick = trough_x + trough_width * index / 5.0;
    tick - label_width / 2.0
}

fn label_text_width(label: &gtk::Label) -> i32 {
    // Widget::measure starts reporting the allocated width once margins are set,
    // which would slide the labels every frame. The text layout does not.
    label.create_pango_layout(Some(&label.label())).pixel_size().0
}

fn mark_centers(scale: &gtk::Scale) -> Vec<f64> {
    let mut child = scale.first_child();
    while let Some(widget) = child {
        if widget.css_name() == "marks" {
            let mut centers = Vec::new();
            let mut mark = widget.first_child();
            while let Some(current) = mark {
                if current.width() > 0 {
                    let origin = gtk::graphene::Point::new(0.0, 0.0);
                    if let Some(point) = current.compute_point(scale, &origin) {
                        centers.push(point.x() as f64 + current.width() as f64 / 2.0);
                    }
                }
                mark = current.next_sibling();
            }
            return centers;
        }
        child = widget.next_sibling();
    }
    Vec::new()
}

fn place_tick_labels(centers: &[f64], labels: &[gtk::Label]) -> bool {
    let Some(overlay) = labels.first().and_then(|label| label.parent()) else {
        return false;
    };
    let overlay_width = overlay.width();
    if overlay_width <= 0 {
        return false;
    }
    // The overlay shares its left edge with the scale, so a mark's x is the
    // label's x. margin_end keeps the allocated width equal to the text, which
    // stops the last label being squeezed off its tick.
    // Store, Fast, Normal, Slow sit on stops 1, 2, 4, and 6. Stops 3 and 5 are dots.
    const LABELED: [usize; 4] = [0, 1, 3, 5];
    for (label, index) in labels.iter().zip(LABELED) {
        let Some(center) = centers.get(index) else {
            return false;
        };
        let natural = label_text_width(label);
        if natural <= 0 {
            return false;
        }
        let start = (center - natural as f64 / 2.0).round() as i32;
        label.set_margin_start(start);
        label.set_margin_end(overlay_width - start - natural);
    }
    true
}

#[cfg(test)]
mod layout_tests {
    use super::tick_label_x;

    #[test]
    fn labels_center_on_six_even_ticks() {
        let trough_x = 1.0;
        let trough_width = 300.0;
        let label = 40.0;
        let centers: Vec<f64> = (0..6)
            .map(|index| tick_label_x(trough_x, trough_width, index as f64, label) + label / 2.0)
            .collect();
        assert!((centers[0] - trough_x).abs() < 0.01);
        assert!((centers[5] - (trough_x + trough_width)).abs() < 0.01);
        let step = centers[1] - centers[0];
        for index in 1..5 {
            assert!((centers[index + 1] - centers[index] - step).abs() < 0.01);
        }
        assert!((step - trough_width / 5.0).abs() < 0.01);
        // Store, Fast, Normal, Slow are stops 1, 2, 4, and 6.
        assert!((centers[1] - (trough_x + step)).abs() < 0.01);
        assert!((centers[3] - (trough_x + step * 3.0)).abs() < 0.01);
    }
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
    let _ = cr.select_font_face("Roboto", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    let size = if format.badge().chars().count() > 3 { 6.5 } else { 8.0 };
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

fn draw_stack(cr: &gtk::cairo::Context, width: f64, height: f64) {
    let sheets = [
        (18.0, 8.0, 0.20, 0.20, 0.22),
        (10.0, 22.0, 0.28, 0.28, 0.31),
        (2.0, 36.0, 0.40, 0.42, 0.46),
    ];
    for (x, y, r, g, b) in sheets {
        cr.set_source_rgb(r, g, b);
        rounded(cr, x, y, width - 36.0, height - 48.0, 8.0);
        let _ = cr.fill();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
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

