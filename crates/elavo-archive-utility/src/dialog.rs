//! Dialog windows in the Elavo style: a GTK title bar from the theme, as in
//! System Settings' `popup_window`. Without a client-side title bar labwc
//! would draw its own server-side frame instead.

use gtk::prelude::*;

pub fn window(parent: &impl IsA<gtk::Window>, title: &str, width: i32) -> gtk::Window {
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .transient_for(parent)
        .destroy_with_parent(true)
        .default_width(width)
        .resizable(false)
        .build();
    if let Some(app) = parent.as_ref().application() {
        window.set_application(Some(&app));
    }
    window.set_titlebar(Some(&gtk::HeaderBar::new()));
    window
}

/// The password prompt used by both windows. Return in the field presses
/// Open.
pub struct PasswordPrompt {
    pub window: gtk::Window,
    pub entry: gtk::PasswordEntry,
    pub open: gtk::Button,
}

pub fn password(parent: &impl IsA<gtk::Window>, message: &str) -> PasswordPrompt {
    let window = window(parent, "Password", 340);
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["au-dialog-body"])
        .build();
    let label = gtk::Label::builder()
        .label(message)
        .wrap(true)
        .xalign(0.0)
        .build();
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("Password")
        .activates_default(true)
        .build();
    let cancel = gtk::Button::builder().label("Cancel").build();
    let open = gtk::Button::builder()
        .label("Open")
        .css_classes(["suggested-action"])
        .build();
    let buttons = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    buttons.append(&cancel);
    buttons.append(&open);
    body.append(&label);
    body.append(&entry);
    body.append(&buttons);
    window.set_child(Some(&body));
    window.set_default_widget(Some(&open));
    let window_cancel = window.clone();
    cancel.connect_clicked(move |_| window_cancel.close());
    PasswordPrompt {
        window,
        entry,
        open,
    }
}
