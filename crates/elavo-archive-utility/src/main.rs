//! Archive Utility compresses files and extracts archives.

mod accent;
mod app;
mod cli;
mod extract;
mod prefs;
mod window;

use gtk::gio;
use gtk::glib;

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if cli::wants(&args) {
        std::process::exit(cli::run(&args));
    }
    gio::resources_register_include!("elavo-archive-utility.gresource")
        .expect("compiled resources");
    app::run()
}
