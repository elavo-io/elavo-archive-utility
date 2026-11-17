//! Compress and extract archives for Archive Utility.
//!
//! This crate is plain Rust. Tar archives and single-file gzip, xz, bzip2,
//! and zstd streams are handled in process. 7z and ZIP, including passwords
//! and split volumes, are handed to the `7z` program. RAR is extracted only,
//! with `unar` or `unrar-free`, never with 7-Zip's unRAR code. ISO images
//! are extracted with `7z` or `bsdtar`.
//!
//! A password is written to the tool's standard input, or to its prompt on a
//! private terminal, and is never an argument. It is not written to the
//! settings file.

mod cancel;
mod engine;
mod error;
mod foreign;
mod format;
mod names;
mod process;
mod progress;
mod rar;
mod settings;
mod seven;
mod streams;
mod tarpack;
mod tools;
mod transplant;
mod unarr;

pub use cancel::Cancel;
pub use engine::{compress, extract, CompressRequest, ExtractRequest, Outcome};
pub use error::{Error, Result};
pub use format::{
    all_archives, create_choices, detect, parse_size, CreateChoice, Format, Level, SplitPreset,
    CREATE_FORMATS, SPLIT_PRESETS,
};
pub use names::{
    archive_stem, archive_stem_for, format_bytes, is_macos_metadata, suggest_archive_path,
    suggest_extract_parent,
};
pub use progress::Progress;
pub use settings::{
    config_path, Conflict, FormatOptions, NewFolder, OutputPlace, Settings, Split,
};
pub use tools::{tools, Tools};
