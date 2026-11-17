use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::format::{Format, Level};

/// Where a finished archive is written, and where an extract begins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputPlace {
    /// Next to the original files, or next to the archive.
    Beside,
    Directory(PathBuf),
}

/// What to do when an extracted name is already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    Rename,
    Replace,
    Skip,
}

impl Conflict {
    pub fn id(self) -> &'static str {
        match self {
            Conflict::Rename => "rename",
            Conflict::Replace => "replace",
            Conflict::Skip => "skip",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "replace" => Conflict::Replace,
            "skip" => Conflict::Skip,
            _ => Conflict::Rename,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Conflict::Rename => "Rename the new file",
            Conflict::Replace => "Replace the existing file",
            Conflict::Skip => "Skip the new file",
        }
    }
}

/// Whether extracting creates a folder named after the archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewFolder {
    Always,
    /// Only when the archive has more than one item at the top.
    WhenSeveral,
    Never,
}

impl NewFolder {
    pub fn id(self) -> &'static str {
        match self {
            NewFolder::Always => "always",
            NewFolder::WhenSeveral => "several",
            NewFolder::Never => "never",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "always" => NewFolder::Always,
            "never" => NewFolder::Never,
            _ => NewFolder::WhenSeveral,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NewFolder::Always => "Always",
            NewFolder::WhenSeveral => "When there are several items",
            NewFolder::Never => "Never",
        }
    }
}

/// Volume sizes offered in the window. `None` writes one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    None,
    Mb10,
    Mb100,
    Mb700,
    Dvd,
}

impl Split {
    pub fn id(self) -> &'static str {
        match self {
            Split::None => "none",
            Split::Mb10 => "10mb",
            Split::Mb100 => "100mb",
            Split::Mb700 => "700mb",
            Split::Dvd => "dvd",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "10mb" => Split::Mb10,
            "100mb" => Split::Mb100,
            "700mb" => Split::Mb700,
            "dvd" => Split::Dvd,
            _ => Split::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Split::None => "Don't split",
            Split::Mb10 => "10 MB",
            Split::Mb100 => "100 MB",
            Split::Mb700 => "700 MB",
            Split::Dvd => "4.7 GB",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Split::None => "",
            Split::Mb10 => "10 MB",
            Split::Mb100 => "100 MB",
            Split::Mb700 => "700 MB",
            Split::Dvd => "4.7 GB",
        }
    }

    pub fn from_text(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().replace(' ', "").as_str() {
            "" => Split::None,
            "10mb" | "10m" => Split::Mb10,
            "100mb" | "100m" => Split::Mb100,
            "700mb" | "700m" => Split::Mb700,
            "4.7gb" | "dvd" => Split::Dvd,
            _ => Split::None,
        }
    }

    pub fn bytes(self) -> Option<u64> {
        match self {
            Split::None => None,
            Split::Mb10 => Some(10 * 1024 * 1024),
            Split::Mb100 => Some(100 * 1024 * 1024),
            Split::Mb700 => Some(700 * 1024 * 1024),
            Split::Dvd => Some(4700 * 1024 * 1024),
        }
    }

    pub const CHOICES: &'static [Split] = &[
        Split::None,
        Split::Mb10,
        Split::Mb100,
        Split::Mb700,
        Split::Dvd,
    ];
}

/// Compression choices for one format. The password is never one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatOptions {
    pub level: Level,
    pub split: String,
    pub aes256: bool,
    pub exclude_macos: bool,
    pub verify: bool,
    pub delete_after: bool,
    pub separate: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            level: Level::Normal,
            split: String::new(),
            aes256: false,
            exclude_macos: true,
            verify: false,
            delete_after: false,
            separate: false,
        }
    }
}

/// What Archive Utility remembers between runs. The password is never written here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub format: Format,
    pub level: Level,
    pub encrypt_names: bool,
    pub split: Split,
    pub new_folder: NewFolder,
    pub conflict: Conflict,
    pub open_destination: bool,
    pub output: OutputPlace,
    pub width: i32,
    pub height: i32,
    pub compact: bool,
    /// Per-format options, keyed by the format id (`7z`, `zip`, `gz`, …).
    pub format_options: BTreeMap<String, FormatOptions>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::SevenZ,
            level: Level::Normal,
            encrypt_names: false,
            split: Split::None,
            new_folder: NewFolder::WhenSeveral,
            conflict: Conflict::Rename,
            open_destination: true,
            output: OutputPlace::Beside,
            width: 330,
            height: 370,
            compact: false,
            format_options: BTreeMap::new(),
        }
    }
}

impl Settings {
    pub fn options_of(&self, format: Format) -> FormatOptions {
        let id = format.pill().id();
        if let Some(options) = self.format_options.get(id) {
            return options.clone();
        }
        let mut options = FormatOptions::default();
        if format.pill() == self.format.pill() {
            options.level = self.level;
            options.split = self.split.text().to_string();
        }
        options
    }

    pub fn set_options(&mut self, format: Format, options: FormatOptions) {
        let format = format.pill();
        self.format = format;
        self.level = options.level;
        self.split = Split::from_text(&options.split);
        self.format_options
            .insert(format.id().to_string(), options);
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap_or_default();
        let mut settings = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if let Some(rest) = key.strip_prefix("f.") {
                if let Some((id, field)) = rest.split_once('.') {
                    let entry = settings.format_options.entry(id.to_string()).or_default();
                    match field {
                        "level" => entry.level = Level::from_id(value),
                        "split" => entry.split = value.to_string(),
                        "aes256" => entry.aes256 = value == "true",
                        "forks" => entry.exclude_macos = value != "false",
                        "verify" => entry.verify = value == "true",
                        "delete" => entry.delete_after = value == "true",
                        "separate" => entry.separate = value == "true",
                        _ => {}
                    }
                }
                continue;
            }
            match key {
                "format" => {
                    if let Some(format) = Format::from_id(value) {
                        settings.format = format;
                    }
                }
                "level" => settings.level = Level::from_id(value),
                "encrypt_names" => settings.encrypt_names = value == "true",
                "split" => settings.split = Split::from_id(value),
                "new_folder" => settings.new_folder = NewFolder::from_id(value),
                "conflict" => settings.conflict = Conflict::from_id(value),
                "open_destination" => settings.open_destination = value != "false",
                "output" => {
                    if value == "beside" {
                        settings.output = OutputPlace::Beside;
                    }
                }
                "output_dir" => {
                    if !value.is_empty() {
                        settings.output = OutputPlace::Directory(PathBuf::from(value));
                    }
                }
                "width" => {
                    if let Ok(width) = value.parse::<i32>() {
                        settings.width = width.clamp(280, 4000);
                    }
                }
                "height" => {
                    if let Ok(height) = value.parse::<i32>() {
                        settings.height = height.clamp(280, 4000);
                    }
                }
                "compact" => settings.compact = value == "true",
                _ => {}
            }
        }
        settings
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let dir_line = match &self.output {
            OutputPlace::Beside => String::new(),
            OutputPlace::Directory(dir) => dir.display().to_string(),
        };
        let output = match &self.output {
            OutputPlace::Beside => "beside",
            OutputPlace::Directory(_) => "directory",
        };
        let mut options = String::new();
        for (id, item) in &self.format_options {
            options.push_str(&format!(
                "f.{id}.level={level}\nf.{id}.split={split}\nf.{id}.aes256={aes}\nf.{id}.forks={forks}\nf.{id}.verify={verify}\nf.{id}.delete={delete}\nf.{id}.separate={separate}\n",
                level = item.level.id(),
                split = item.split,
                aes = item.aes256,
                forks = item.exclude_macos,
                verify = item.verify,
                delete = item.delete_after,
                separate = item.separate,
            ));
        }
        let body = format!(
            "\
# Archive Utility settings. The password is never stored.
format={format}
level={level}
encrypt_names={encrypt}
split={split}
new_folder={folder}
conflict={conflict}
open_destination={open}
output={output}
output_dir={dir}
width={width}
height={height}
compact={compact}
{options}",
            format = self.format.id(),
            level = self.level.id(),
            encrypt = self.encrypt_names,
            split = self.split.id(),
            folder = self.new_folder.id(),
            conflict = self.conflict.id(),
            open = self.open_destination,
            dir = dir_line,
            width = self.width,
            height = self.height,
            compact = self.compact,
        );
        let tmp = path.with_extension("ini.partial");
        fs::write(&tmp, body)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// `~/.config/elavo-archive-utility/config.ini`, or `$XDG_CONFIG_HOME/elavo-archive-utility/config.ini`.
/// `ELAVO_ARCHIVE_UTILITY_CONFIG` overrides the file, for a private session.
pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("ELAVO_ARCHIVE_UTILITY_CONFIG") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|p| !p.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("elavo-archive-utility").join("config.ini")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_skip_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let mut settings = Settings::default();
        settings.format = Format::TarGz;
        settings.level = Level::Slowest;
        settings.encrypt_names = true;
        settings.split = Split::Mb100;
        settings.new_folder = NewFolder::Always;
        settings.conflict = Conflict::Skip;
        settings.open_destination = false;
        settings.output = OutputPlace::Directory(PathBuf::from("/tmp/out"));
        settings.compact = true;
        settings.width = 800;
        settings.height = 500;
        settings.save_to(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            !text.lines().any(|line| {
                let key = line.split('=').next().unwrap_or("").trim();
                key.eq_ignore_ascii_case("password")
            }),
            "settings must not store a password"
        );
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded, settings);

        fs::write(&path, "format=nope\nlevel=fast\nextra=1\n").unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.format, Format::SevenZ);
        assert_eq!(loaded.level, Level::Fastest);
    }
}
