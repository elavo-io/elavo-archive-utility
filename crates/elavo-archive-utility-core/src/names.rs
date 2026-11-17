use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::format::Format;
use crate::settings::OutputPlace;

/// Bytes in a short form for the status line: "980 bytes", "12.4 MB".
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 10.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// AppleDouble files, Finder metadata, and the `__MACOSX` directory.
pub fn is_macos_metadata(rel: &str) -> bool {
    rel.split(['/', '\\']).any(|part| {
        !part.is_empty() && (part == ".DS_Store" || part == "__MACOSX" || part.starts_with("._"))
    })
}

/// A relative path an archive is allowed to create. Absolute paths and
/// `..` are refused so a hostile archive cannot write outside the folder.
pub fn is_safe_rel(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    path.components().all(|component| {
        matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    })
}

pub fn archive_stem(file_name: &str) -> String {
    let lower = file_name.to_lowercase();
    const COMPOUND: &[&str] = &[
        ".tar.lzma",
        ".tar.zstd",
        ".tar.lz4",
        ".tar.lrz",
        ".tar.bz2",
        ".tar.zst",
        ".tar.gz",
        ".tar.xz",
        ".tar.lz",
        ".tar.br",
        ".tar.sz",
        ".tar.z",
        ".tbz2",
        ".tzst",
        ".tlz4",
        ".tgz",
        ".tbz",
        ".tb2",
        ".txz",
        ".taz",
    ];
    for suffix in COMPOUND {
        if let Some(stripped) = strip_suffix_ci(file_name, &lower, suffix) {
            return sanitize_stem(stripped);
        }
    }
    if let Some((rest, num)) = lower.rsplit_once('.') {
        if num.len() == 3
            && num.chars().all(|c| c.is_ascii_digit())
            && (rest.ends_with(".7z") || rest.ends_with(".zip"))
        {
            let cut = file_name.len() - num.len() - 1;
            let without_part = &file_name[..cut];
            return archive_stem(without_part);
        }
    }
    let stem = Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.to_string());
    sanitize_stem(&stem)
}

fn strip_suffix_ci<'a>(original: &'a str, lower: &str, suffix: &str) -> Option<&'a str> {
    if lower.ends_with(suffix) && original.len() > suffix.len() {
        Some(&original[..original.len() - suffix.len()])
    } else {
        None
    }
}

fn sanitize_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| *c != '/' && *c != '\\' && *c != '\0')
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "Archive".to_string()
    } else {
        cleaned.to_string()
    }
}

/// The base name of a new archive, before the format's extension.
pub fn archive_stem_for(inputs: &[PathBuf]) -> String {
    if inputs.is_empty() {
        return "Archive".to_string();
    }
    if inputs.len() == 1 {
        let path = &inputs[0];
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Archive".to_string());
        if path.is_dir() {
            return sanitize_stem(&name);
        }
        let stem = Path::new(&name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(name);
        return sanitize_stem(&stem);
    }
    if let Some(parent) = common_parent(inputs) {
        if let Some(name) = parent.file_name() {
            let text = name.to_string_lossy();
            if !text.is_empty() && text != "/" {
                return sanitize_stem(&text);
            }
        }
    }
    "Archive".to_string()
}

pub fn common_parent(inputs: &[PathBuf]) -> Option<PathBuf> {
    let parent = inputs.first()?.parent()?.to_path_buf();
    if parent.as_os_str().is_empty() {
        return None;
    }
    if inputs
        .iter()
        .all(|path| path.parent() == Some(parent.as_path()))
    {
        Some(parent)
    } else {
        None
    }
}

/// Folder a new archive is written into.
pub fn output_directory(inputs: &[PathBuf], place: &OutputPlace) -> PathBuf {
    match place {
        OutputPlace::Directory(dir) => dir.clone(),
        OutputPlace::Beside => inputs
            .first()
            .and_then(|path| path.parent())
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

/// Folder an extract starts from, before the "new folder" rule is applied.
pub fn suggest_extract_parent(archive: &Path, place: &OutputPlace) -> PathBuf {
    match place {
        OutputPlace::Directory(dir) => dir.clone(),
        OutputPlace::Beside => archive
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

pub fn suggest_archive_path(inputs: &[PathBuf], format: Format, place: &OutputPlace) -> PathBuf {
    let dir = output_directory(inputs, place);
    let stem = if format.is_stream() {
        inputs
            .first()
            .and_then(|path| path.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string())
    } else {
        archive_stem_for(inputs)
    };
    unused_path(&dir, &stem, format.extension())
}

/// `Photos.tar.gz`, then `Photos 2.tar.gz`, skipping names that are already taken.
pub fn unused_path(dir: &Path, stem: &str, extension: &str) -> PathBuf {
    for n in 1..10_000u32 {
        let name = if n == 1 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem} {n}.{extension}")
        };
        let path = dir.join(&name);
        let volume = dir.join(format!("{name}.001"));
        if !path.exists() && !volume.exists() {
            return path;
        }
    }
    dir.join(format!("{stem} {}.{extension}", std::process::id()))
}

/// Names of the items as they should appear at the top of an archive.
pub fn top_names(inputs: &[PathBuf]) -> Vec<String> {
    let mut used = HashSet::new();
    let mut names = Vec::with_capacity(inputs.len());
    for input in inputs {
        let raw = input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "item".to_string());
        let raw = if raw.is_empty() || raw == "." || raw == ".." || raw.contains('\0') {
            "item".to_string()
        } else {
            raw
        };
        let mut name = raw.clone();
        let mut n = 2u32;
        while !used.insert(name.clone()) {
            name = format!("{raw} ({n})");
            n += 1;
        }
        names.push(name);
    }
    names
}

/// First path component of each archive member. Used to decide whether
/// extracting should create a new folder.
pub fn toplevel_count(paths: impl IntoIterator<Item = impl AsRef<str>>) -> usize {
    let mut names = HashSet::new();
    for path in paths {
        let path = path.as_ref().replace('\\', "/");
        let path = path.trim_matches('/');
        if path.is_empty() {
            continue;
        }
        if let Some(first) = path.split('/').next() {
            if first != "." && first != ".." && !first.is_empty() {
                names.insert(first.to_string());
            }
        }
    }
    names.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_drop_compound_suffixes() {
        assert_eq!(archive_stem("Photos.tar.gz"), "Photos");
        assert_eq!(archive_stem("Notes.tgz"), "Notes");
        assert_eq!(archive_stem("disk.iso"), "disk");
        assert_eq!(archive_stem("vol.7z.001"), "vol");
        assert_eq!(archive_stem("hello.txt.gz"), "hello.txt");
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        assert!(is_safe_rel(Path::new("a/b.txt")));
        assert!(is_safe_rel(Path::new("./a")));
        assert!(!is_safe_rel(Path::new("../x")));
        assert!(!is_safe_rel(Path::new("/etc/passwd")));
        assert!(!is_safe_rel(Path::new("a/../../x")));
    }

    #[test]
    fn unused_path_skips_existing_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Photos.tar.gz"), b"x").unwrap();
        let path = unused_path(dir.path(), "Photos", "tar.gz");
        assert_eq!(path.file_name().unwrap(), "Photos 2.tar.gz");
    }

    #[test]
    fn several_items_use_the_shared_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let stem = archive_stem_for(&[a, b]);
        assert_eq!(stem, dir.path().file_name().unwrap().to_string_lossy());
    }

    #[test]
    fn sizes_read_in_units_of_1024() {
        assert_eq!(format_bytes(980), "980 bytes");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(12 * 1024 * 1024), "12 MB");
    }
}
