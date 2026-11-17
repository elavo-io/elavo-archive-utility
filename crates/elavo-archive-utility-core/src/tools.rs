use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Programs Archive Utility may call. Formats implemented in process do not need
/// any of these. Each one is optional: a missing program becomes a message
/// that names what to install.
#[derive(Clone, Debug)]
pub struct Tools {
    pub seven_zip: Option<PathBuf>,
    pub unar: Option<PathBuf>,
    pub lsar: Option<PathBuf>,
    pub unrar_free: Option<PathBuf>,
    pub bsdtar: Option<PathBuf>,
    pub zip: Option<PathBuf>,
    pub unzip: Option<PathBuf>,
    pub bwrap: Option<PathBuf>,
    pub bwrap_works: bool,
    pub lzip: Option<PathBuf>,
    pub lz4: Option<PathBuf>,
    pub brotli: Option<PathBuf>,
    pub lrzip: Option<PathBuf>,
    pub gzip: Option<PathBuf>,
    pub xz: Option<PathBuf>,
    pub lzma: Option<PathBuf>,
    pub compress: Option<PathBuf>,
    pub uncompress: Option<PathBuf>,
    pub cabextract: Option<PathBuf>,
    pub innoextract: Option<PathBuf>,
    pub dpkg_deb: Option<PathBuf>,
    pub iso_creator: Option<PathBuf>,
    pub iso_kind: IsoKind,
    /// `dmg build` from libdmg-hfsplus, when that program is installed.
    pub dmg_build: Option<PathBuf>,
    pub ar: Option<PathBuf>,
    pub cpio: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsoKind {
    None,
    Xorriso,
    Genisoimage,
}

impl Tools {
    pub fn discover() -> Self {
        let seven_zip = find_seven_zip();
        let bwrap = which("bwrap");
        let bwrap_works = bwrap.as_ref().is_some_and(|path| probe_bwrap(path));
        let xorriso = which("xorriso");
        let genisoimage = which("genisoimage").or_else(|| which("mkisofs"));
        let (iso_creator, iso_kind) = if let Some(path) = xorriso {
            (Some(path), IsoKind::Xorriso)
        } else if let Some(path) = genisoimage {
            (Some(path), IsoKind::Genisoimage)
        } else {
            (None, IsoKind::None)
        };
        Self {
            seven_zip,
            unar: which("unar"),
            lsar: which("lsar"),
            unrar_free: which("unrar-free").or_else(|| which("unrar")),
            bsdtar: which("bsdtar"),
            zip: which("zip"),
            unzip: which("unzip"),
            bwrap,
            bwrap_works,
            lzip: which("lzip"),
            lz4: which("lz4"),
            brotli: which("brotli"),
            lrzip: which("lrzip"),
            gzip: which("gzip"),
            xz: which("xz"),
            lzma: which("lzma"),
            compress: which("compress"),
            uncompress: which("uncompress"),
            cabextract: which("cabextract"),
            innoextract: which("innoextract"),
            dpkg_deb: which("dpkg-deb"),
            iso_creator,
            iso_kind,
            dmg_build: which("dmg").filter(|path| dmg_can_build(path)),
            ar: which("ar"),
            cpio: which("cpio"),
        }
    }
}

pub fn tools() -> &'static Tools {
    static TOOLS: OnceLock<Tools> = OnceLock::new();
    TOOLS.get_or_init(Tools::discover)
}

fn find_seven_zip() -> Option<PathBuf> {
    // Prefer the real binary. Debian's `/usr/bin/7z` is a shell script that
    // execs this, and a script would show up as an extra process.
    for candidate in ["/usr/lib/7zip/7z", "/usr/libexec/p7zip/7z"] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    which("7zz")
        .or_else(|| which("7z"))
        .or_else(|| which("7za"))
}

pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn probe_bwrap(path: &Path) -> bool {
    std::process::Command::new(path)
        .args([
            "--unshare-all",
            "--die-with-parent",
            "--ro-bind",
            "/",
            "/",
            "--",
            "/usr/bin/true",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// libdmg-hfsplus's `dmg` prints a usage line that mentions `build`.
/// Other programs named `dmg` are left unused.
fn dmg_can_build(path: &Path) -> bool {
    let output = std::process::Command::new(path)
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .output();
    let Ok(output) = output else {
        return false;
    };
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let lower = text.to_lowercase();
    lower.contains("build") && (lower.contains("dmg") || lower.contains("hfs"))
}

/// Hide passwords before a command is written to a log.
pub fn redact(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut hide_next = false;
    for arg in args {
        if hide_next {
            out.push("******".to_string());
            hide_next = false;
            continue;
        }
        if arg == "-p" || arg == "--password" || arg == "-password" || arg == "-P" {
            out.push(arg.clone());
            hide_next = true;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("-p") {
            if !rest.is_empty() && !arg.starts_with("-proc") {
                out.push("-p******".to_string());
                continue;
            }
        }
        if let Some((_, _)) = arg.split_once("--password=") {
            out.push("--password=******".to_string());
            continue;
        }
        out.push(arg.clone());
    }
    out
}

/// True when an argument is the password itself or a switch glued to it.
pub fn args_contain_secret(args: &[String], secret: &str) -> bool {
    if secret.is_empty() {
        return false;
    }
    args.iter().any(|arg| {
        arg == secret
            || arg == &format!("-p{secret}")
            || arg == &format!("-P{secret}")
            || arg == &format!("--password={secret}")
            || arg.strip_prefix("-p").is_some_and(|rest| rest == secret)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_redacted() {
        let args = vec![
            "a".into(),
            "-psecret".into(),
            "-p".into(),
            "other".into(),
            "--password".into(),
            "nope".into(),
            "--proc".into(),
            "/proc".into(),
        ];
        let redacted = redact(&args);
        let joined = redacted.join(" ");
        assert!(!joined.contains("secret"));
        assert!(!joined.contains("other"));
        assert!(!joined.contains("nope"));
        assert!(joined.contains("/proc"));
    }

    #[test]
    fn a_tool_on_path_is_found() {
        if std::path::Path::new("/usr/bin/lzip").is_file() {
            assert!(tools().lzip.is_some(), "lzip is installed but was not found");
        }
        if std::path::Path::new("/usr/bin/lrzip").is_file() {
            assert!(tools().lrzip.is_some(), "lrzip is installed but was not found");
        }
    }

    #[test]
    fn a_glued_password_is_a_leak() {
        let secret = "correct horse";
        assert!(args_contain_secret(
            &["-pcorrect horse".into()],
            secret
        ));
        assert!(!args_contain_secret(&["-p".into(), "archive.7z".into()], secret));
    }
}
