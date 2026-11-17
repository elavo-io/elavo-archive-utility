//! Optional RAR extraction through libunarr (LGPL-3), loaded at runtime.
//! The library is not linked into Archive Utility, so a missing `libunarr.so.1`
//! only disables this path. Encrypted RAR archives still need `unar`.

use std::ffi::{CStr, CString};
use std::fs::{self, File};
use std::io::Write;
use std::os::raw::{c_char, c_void};
use std::path::Path;
use std::sync::OnceLock;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::names::is_safe_rel;
use crate::settings::Conflict;
use crate::transplant::{checked_join, prepare_dirs, unique_path};

type ArStream = c_void;
type ArArchive = c_void;

struct Unarr {
    open_file: unsafe extern "C" fn(*const c_char) -> *mut ArStream,
    open_rar: unsafe extern "C" fn(*mut ArStream) -> *mut ArArchive,
    parse: unsafe extern "C" fn(*mut ArArchive) -> bool,
    at_eof: unsafe extern "C" fn(*mut ArArchive) -> bool,
    name: unsafe extern "C" fn(*mut ArArchive) -> *const c_char,
    size: unsafe extern "C" fn(*mut ArArchive) -> usize,
    uncompress: unsafe extern "C" fn(*mut ArArchive, *mut c_void, usize) -> bool,
    close_archive: unsafe extern "C" fn(*mut ArArchive),
    close_stream: unsafe extern "C" fn(*mut ArStream),
    _library: *mut c_void,
}

unsafe impl Send for Unarr {}
unsafe impl Sync for Unarr {}

pub fn available() -> bool {
    load().is_some()
}

pub fn extract(
    archive: &Path,
    dest: &Path,
    conflict: Conflict,
    cancel: &Cancel,
) -> Result<u64> {
    let Some(lib) = load() else {
        return Err(Error::ToolMissing {
            tool: "unar",
            purpose: "open this RAR archive".into(),
        });
    };
    let path = CString::new(archive.display().to_string()).map_err(|_| {
        Error::Failed("The archive path cannot be passed to libunarr.".into())
    })?;
    unsafe {
        let stream = (lib.open_file)(path.as_ptr());
        if stream.is_null() {
            return Err(Error::Failed("Couldn't open the RAR archive.".into()));
        }
        let opened = (lib.open_rar)(stream);
        if opened.is_null() {
            (lib.close_stream)(stream);
            return Err(Error::Failed(
                "libunarr could not read this RAR archive.".into(),
            ));
        }
        let result = extract_entries(lib, opened, dest, conflict, cancel);
        (lib.close_archive)(opened);
        (lib.close_stream)(stream);
        result
    }
}

unsafe fn extract_entries(
    lib: &Unarr,
    archive: *mut ArArchive,
    dest: &Path,
    conflict: Conflict,
    cancel: &Cancel,
) -> Result<u64> {
    fs::create_dir_all(dest)?;
    let mut files = 0u64;
    loop {
        cancel.check()?;
        if !(lib.parse)(archive) {
            break;
        }
        let name_ptr = (lib.name)(archive);
        if name_ptr.is_null() {
            continue;
        }
        let name = CStr::from_ptr(name_ptr).to_string_lossy().replace('\\', "/");
        let rel = Path::new(name.trim_matches('/'));
        if rel.as_os_str().is_empty() {
            continue;
        }
        if !is_safe_rel(rel) {
            return Err(Error::UnsafePath(name));
        }
        if name.ends_with('/') {
            let dir = checked_join(dest, rel)?;
            fs::create_dir_all(&dir)?;
            continue;
        }
        let size = (lib.size)(archive);
        let dest_path = checked_join(dest, rel)?;
        let Some(dest_path) = place(&dest_path, conflict)? else {
            // Still have to consume the compressed bytes.
            let mut sink = vec![0u8; 64 * 1024];
            let mut left = size;
            while left > 0 {
                let n = left.min(sink.len());
                if !(lib.uncompress)(archive, sink.as_mut_ptr().cast(), n) {
                    return Err(Error::Failed(format!("Couldn't read {name}.")));
                }
                left -= n;
            }
            continue;
        };
        if let Some(parent) = dest_path.parent() {
            prepare_dirs(dest, parent.strip_prefix(dest).unwrap_or(Path::new("")))?;
        }
        let mut file = File::create(&dest_path)?;
        let mut left = size;
        let mut buf = vec![0u8; 64 * 1024];
        while left > 0 {
            cancel.check()?;
            let n = left.min(buf.len());
            if !(lib.uncompress)(archive, buf.as_mut_ptr().cast(), n) {
                return Err(Error::Failed(format!("Couldn't read {name}.")));
            }
            file.write_all(&buf[..n])?;
            left -= n;
        }
        files += 1;
    }
    if !(lib.at_eof)(archive) && files == 0 {
        return Err(Error::Failed(
            "libunarr could not read this RAR archive.".into(),
        ));
    }
    Ok(files)
}

fn place(path: &Path, conflict: Conflict) -> Result<Option<std::path::PathBuf>> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(Some(path.to_path_buf()));
    }
    match conflict {
        Conflict::Skip => Ok(None),
        Conflict::Replace => {
            if let Ok(meta) = fs::symlink_metadata(path) {
                if meta.is_dir() && !meta.file_type().is_symlink() {
                    return Err(Error::Failed(format!(
                        "{} is a folder, so it was left in place.",
                        path.display()
                    )));
                }
            }
            let _ = fs::remove_file(path);
            Ok(Some(path.to_path_buf()))
        }
        Conflict::Rename => Ok(Some(unique_path(path))),
    }
}

fn load() -> Option<&'static Unarr> {
    static LOADED: OnceLock<Option<Unarr>> = OnceLock::new();
    LOADED.get_or_init(load_library).as_ref()
}

fn load_library() -> Option<Unarr> {
    unsafe {
        let mut library = std::ptr::null_mut();
        for name in ["libunarr.so.1", "libunarr.so"] {
            let name = CString::new(name).ok()?;
            library = libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
            if !library.is_null() {
                break;
            }
        }
        if library.is_null() {
            return None;
        }
        let symbol = |item: &[u8]| -> Option<*mut c_void> {
            let ptr = libc::dlsym(library, item.as_ptr().cast());
            if ptr.is_null() {
                None
            } else {
                Some(ptr)
            }
        };
        macro_rules! fun {
            ($name:literal, $ty:ty) => {{
                let ptr = symbol(concat!($name, "\0").as_bytes())?;
                std::mem::transmute::<*mut c_void, $ty>(ptr)
            }};
        }
        Some(Unarr {
            open_file: fun!("ar_open_file", unsafe extern "C" fn(*const c_char) -> *mut ArStream),
            open_rar: fun!(
                "ar_open_rar_archive",
                unsafe extern "C" fn(*mut ArStream) -> *mut ArArchive
            ),
            parse: fun!("ar_parse_entry", unsafe extern "C" fn(*mut ArArchive) -> bool),
            at_eof: fun!("ar_at_eof", unsafe extern "C" fn(*mut ArArchive) -> bool),
            name: fun!(
                "ar_entry_get_name",
                unsafe extern "C" fn(*mut ArArchive) -> *const c_char
            ),
            size: fun!("ar_entry_get_size", unsafe extern "C" fn(*mut ArArchive) -> usize),
            uncompress: fun!(
                "ar_entry_uncompress",
                unsafe extern "C" fn(*mut ArArchive, *mut c_void, usize) -> bool
            ),
            close_archive: fun!("ar_close_archive", unsafe extern "C" fn(*mut ArArchive)),
            close_stream: fun!("ar_close", unsafe extern "C" fn(*mut ArStream)),
            _library: library,
        })
    }
}
