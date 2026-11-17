//! Preferences. The password is never one of these values.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gio;
use gtk::prelude::*;

use elavo_archive_utility_core::{Conflict, NewFolder, OutputPlace, Settings};

pub fn present(
    parent: &impl IsA<gtk::Window>,
    settings: Rc<RefCell<Settings>>,
    changed: impl Fn() + 'static,
) {
    let changed = Rc::new(changed);
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Preferences")
        .default_width(460)
        .css_classes(["au-prefs"])
        .build();

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .css_classes(["au-prefs-body"])
        .build();

    let open = gtk::CheckButton::builder()
        .label("Open the destination folder when finished")
        .active(settings.borrow().open_destination)
        .build();
    let encrypt = gtk::CheckButton::builder()
        .label("Encrypt 7z file names when a password is set")
        .active(settings.borrow().encrypt_names)
        .build();
    let folder = gtk::DropDown::from_strings(&[
        NewFolder::WhenSeveral.label(),
        NewFolder::Always.label(),
        NewFolder::Never.label(),
    ]);
    folder.set_selected(match settings.borrow().new_folder {
        NewFolder::WhenSeveral => 0,
        NewFolder::Always => 1,
        NewFolder::Never => 2,
    });
    let conflict = gtk::DropDown::from_strings(&[
        Conflict::Rename.label(),
        Conflict::Replace.label(),
        Conflict::Skip.label(),
    ]);
    conflict.set_selected(match settings.borrow().conflict {
        Conflict::Rename => 0,
        Conflict::Replace => 1,
        Conflict::Skip => 2,
    });

    let beside = gtk::CheckButton::builder()
        .label("Beside the original files")
        .active(matches!(settings.borrow().output, OutputPlace::Beside))
        .build();
    let chosen = gtk::CheckButton::builder()
        .label("In a folder I choose")
        .group(&beside)
        .active(matches!(
            settings.borrow().output,
            OutputPlace::Directory(_)
        ))
        .build();
    let dir_label = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["au-dim"])
        .label(match &settings.borrow().output {
            OutputPlace::Directory(dir) => dir.display().to_string(),
            OutputPlace::Beside => "No folder chosen yet".to_string(),
        })
        .build();
    let choose = gtk::Button::builder().label("Choose Folder").build();

    body.append(&section("When an archive is finished"));
    body.append(&card(&open));
    body.append(&card(&encrypt));
    body.append(&section("Extracting"));
    body.append(&card_rows(&[
        ("New folder", folder.upcast_ref()),
        ("When a name is taken", conflict.upcast_ref()),
    ]));
    body.append(&section("Where new archives go"));
    let place = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["au-card"])
        .build();
    place.append(&beside);
    place.append(&chosen);
    let dir_row = gtk::Box::builder().spacing(8).build();
    dir_label.set_hexpand(true);
    dir_row.append(&dir_label);
    dir_row.append(&choose);
    place.append(&dir_row);
    body.append(&place);

    let close = gtk::Button::builder()
        .label("Close")
        .halign(gtk::Align::End)
        .css_classes(["au-primary", "suggested-action"])
        .build();
    body.append(&close);
    window.set_child(Some(&body));

    let settings_open = settings.clone();
    open.connect_toggled(move |button| {
        settings_open.borrow_mut().open_destination = button.is_active();
        let _ = settings_open.borrow().save();
    });
    let settings_encrypt = settings.clone();
    encrypt.connect_toggled(move |button| {
        settings_encrypt.borrow_mut().encrypt_names = button.is_active();
        let _ = settings_encrypt.borrow().save();
    });
    let settings_folder = settings.clone();
    let changed_folder = changed.clone();
    folder.connect_selected_notify(move |dropdown| {
        settings_folder.borrow_mut().new_folder = match dropdown.selected() {
            1 => NewFolder::Always,
            2 => NewFolder::Never,
            _ => NewFolder::WhenSeveral,
        };
        let _ = settings_folder.borrow().save();
        changed_folder();
    });
    let settings_conflict = settings.clone();
    conflict.connect_selected_notify(move |dropdown| {
        settings_conflict.borrow_mut().conflict = match dropdown.selected() {
            1 => Conflict::Replace,
            2 => Conflict::Skip,
            _ => Conflict::Rename,
        };
        let _ = settings_conflict.borrow().save();
    });

    let settings_beside = settings.clone();
    let changed_beside = changed.clone();
    beside.connect_toggled(move |button| {
        if button.is_active() {
            settings_beside.borrow_mut().output = OutputPlace::Beside;
            let _ = settings_beside.borrow().save();
            changed_beside();
        }
    });

    let settings_choose = settings.clone();
    let changed_choose = changed.clone();
    let dir_label_choose = dir_label.clone();
    let chosen_choose = chosen.clone();
    let window_choose = window.clone();
    choose.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::new();
        dialog.set_title("Folder for new archives");
        dialog.set_modal(true);
        let settings = settings_choose.clone();
        let changed = changed_choose.clone();
        let dir_label = dir_label_choose.clone();
        let chosen = chosen_choose.clone();
        dialog.select_folder(
            Some(&window_choose),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        dir_label.set_label(&path.display().to_string());
                        settings.borrow_mut().output = OutputPlace::Directory(path);
                        let _ = settings.borrow().save();
                        chosen.set_active(true);
                        changed();
                    }
                }
            },
        );
    });

    let window_close = window.clone();
    close.connect_clicked(move |_| window_close.close());
    window.present();
}

fn section(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["au-section"])
        .build()
}

fn card(child: &impl IsA<gtk::Widget>) -> gtk::Box {
    let box_ = gtk::Box::builder().css_classes(["au-card"]).build();
    box_.append(child);
    box_
}

fn card_rows(rows: &[(&str, &gtk::Widget)]) -> gtk::Grid {
    let grid = gtk::Grid::builder()
        .column_spacing(12)
        .row_spacing(10)
        .css_classes(["au-card"])
        .build();
    for (index, (label, widget)) in rows.iter().enumerate() {
        let label = gtk::Label::builder()
            .label(*label)
            .xalign(0.0)
            .hexpand(true)
            .build();
        grid.attach(&label, 0, index as i32, 1, 1);
        grid.attach(*widget, 1, index as i32, 1, 1);
    }
    grid
}
