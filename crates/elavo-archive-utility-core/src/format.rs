use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::tools::{self, Tools};

/// An archive or compression format Archive Utility knows.
///
/// Stream formats (gzip, zstd, lzip, …) pack one file. Several files or a
/// folder are wrapped in tar first. RAR and the disc and installer formats
/// are extracted only, with whatever free tool is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    SevenZ,
    Zip,
    Tar,
    TarGz,
    TarXz,
    TarBz2,
    TarZst,
    TarLz,
    TarLz4,
    TarBr,
    TarLrz,
    TarSz,
    TarZ,
    TarLzma,
    Gz,
    Xz,
    Bz2,
    Zst,
    Lzip,
    Lz4,
    Brotli,
    Lrzip,
    Snappy,
    Rar,
    Iso,
    Wim,
    Dmg,
    Exe,
    Msi,
    Cab,
    Lzh,
    Arj,
    Cpio,
    Rpm,
    Deb,
    Xar,
    Ar,
    Chm,
    Compress,
    Lzma,
    SquashFs,
    Vhd,
    Vmdk,
    Qcow,
    Apk,
    Jar,
    Ipa,
}

/// Formats shown in the format menu, in that order. DMG is added only when
/// a free program that can create one is installed.
pub const CREATE_FORMATS: &[Format] = &[
    Format::SevenZ,
    Format::Zip,
    Format::Tar,
    Format::Zst,
    Format::Gz,
    Format::Bz2,
    Format::Xz,
    Format::Lzip,
    Format::Lz4,
    Format::Brotli,
    Format::Lrzip,
    Format::Snappy,
    Format::Wim,
    Format::Iso,
];

/// One row of the format menu.
#[derive(Clone, Copy, Debug)]
pub struct CreateChoice {
    pub format: Format,
    pub available: bool,
    /// Shown when the row is greyed out.
    pub hint: &'static str,
}

impl Format {
    /// Short name drawn on the format pill and in the menu.
    pub fn label(self) -> &'static str {
        match self {
            Format::SevenZ => "7Z",
            Format::Zip => "ZIP",
            Format::Tar | Format::TarGz | Format::TarXz | Format::TarBz2 | Format::TarZst
            | Format::TarLz | Format::TarLz4 | Format::TarBr | Format::TarLrz | Format::TarSz
            | Format::TarZ | Format::TarLzma => "TAR",
            Format::Gz => "GZIP",
            Format::Xz => "XZ",
            Format::Bz2 => "BZIP2",
            Format::Zst => "ZSTD",
            Format::Lzip => "LZIP",
            Format::Lz4 => "LZ4",
            Format::Brotli => "BROTLI",
            Format::Lrzip => "LRZIP",
            Format::Snappy => "SNAPPY",
            Format::Rar => "RAR",
            Format::Iso => "ISO",
            Format::Wim => "WIM",
            Format::Dmg => "DMG",
            Format::Exe => "EXE",
            Format::Msi => "MSI",
            Format::Cab => "CAB",
            Format::Lzh => "LZH",
            Format::Arj => "ARJ",
            Format::Cpio => "CPIO",
            Format::Rpm => "RPM",
            Format::Deb => "DEB",
            Format::Xar => "XAR",
            Format::Ar => "AR",
            Format::Chm => "CHM",
            Format::Compress => "Z",
            Format::Lzma => "LZMA",
            Format::SquashFs => "SQUASHFS",
            Format::Vhd => "VHD",
            Format::Vmdk => "VMDK",
            Format::Qcow => "QCOW",
            Format::Apk => "APK",
            Format::Jar => "JAR",
            Format::Ipa => "IPA",
        }
    }

    /// Letters drawn on the file icon.
    pub fn badge(self) -> &'static str {
        match self {
            Format::SevenZ => "7Z",
            Format::Zip | Format::Apk | Format::Jar | Format::Ipa => "ZIP",
            Format::Tar
            | Format::TarGz
            | Format::TarXz
            | Format::TarBz2
            | Format::TarZst
            | Format::TarLz
            | Format::TarLz4
            | Format::TarBr
            | Format::TarLrz
            | Format::TarSz
            | Format::TarZ
            | Format::TarLzma => "TAR",
            Format::Gz => "GZ",
            Format::Xz => "XZ",
            Format::Bz2 => "BZ2",
            Format::Zst => "ZST",
            Format::Lzip => "LZ",
            Format::Lz4 => "LZ4",
            Format::Brotli => "BR",
            Format::Lrzip => "LRZ",
            Format::Snappy => "SZ",
            Format::Rar => "RAR",
            Format::Iso => "ISO",
            Format::Wim => "WIM",
            Format::Dmg => "DMG",
            Format::Exe => "EXE",
            Format::Msi => "MSI",
            Format::Cab => "CAB",
            Format::Lzh => "LZH",
            Format::Arj => "ARJ",
            Format::Cpio => "CPIO",
            Format::Rpm => "RPM",
            Format::Deb => "DEB",
            Format::Xar => "PKG",
            Format::Ar => "AR",
            Format::Chm => "CHM",
            Format::Compress => "Z",
            Format::Lzma => "LZMA",
            Format::SquashFs => "SQSH",
            Format::Vhd => "VHD",
            Format::Vmdk => "VMDK",
            Format::Qcow => "QCOW",
        }
    }

    /// Icon colour, as sRGB channels in 0.0–1.0. Drawn by Archive Utility, not taken
    /// from another archiver.
    pub fn badge_rgb(self) -> (f64, f64, f64) {
        match self.pill() {
            Format::SevenZ => (0.24, 0.40, 0.86),
            Format::Zip => (0.94, 0.54, 0.14),
            Format::Tar => (0.55, 0.43, 0.33),
            Format::Zst => (0.10, 0.65, 0.65),
            Format::Gz => (0.24, 0.61, 0.31),
            Format::Bz2 => (0.48, 0.37, 0.65),
            Format::Xz => (0.18, 0.44, 0.86),
            Format::Lzip => (0.77, 0.36, 0.15),
            Format::Lz4 => (0.82, 0.23, 0.29),
            Format::Brotli => (0.88, 0.35, 0.17),
            Format::Lrzip => (0.36, 0.42, 0.75),
            Format::Snappy => (0.90, 0.70, 0.15),
            Format::Wim => (0.17, 0.48, 0.73),
            Format::Dmg => (0.42, 0.45, 0.50),
            Format::Iso => (0.29, 0.55, 0.23),
            Format::Rar => (0.70, 0.22, 0.45),
            _ => (0.35, 0.40, 0.48),
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::SevenZ => "7z",
            Format::Zip => "zip",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarXz => "tar.xz",
            Format::TarBz2 => "tar.bz2",
            Format::TarZst => "tar.zst",
            Format::TarLz => "tar.lz",
            Format::TarLz4 => "tar.lz4",
            Format::TarBr => "tar.br",
            Format::TarLrz => "tar.lrz",
            Format::TarSz => "tar.sz",
            Format::TarZ => "tar.Z",
            Format::TarLzma => "tar.lzma",
            Format::Gz => "gz",
            Format::Xz => "xz",
            Format::Bz2 => "bz2",
            Format::Zst => "zst",
            Format::Lzip => "lz",
            Format::Lz4 => "lz4",
            Format::Brotli => "br",
            Format::Lrzip => "lrz",
            Format::Snappy => "sz",
            Format::Rar => "rar",
            Format::Iso => "iso",
            Format::Wim => "wim",
            Format::Dmg => "dmg",
            Format::Exe => "exe",
            Format::Msi => "msi",
            Format::Cab => "cab",
            Format::Lzh => "lzh",
            Format::Arj => "arj",
            Format::Cpio => "cpio",
            Format::Rpm => "rpm",
            Format::Deb => "deb",
            Format::Xar => "pkg",
            Format::Ar => "a",
            Format::Chm => "chm",
            Format::Compress => "Z",
            Format::Lzma => "lzma",
            Format::SquashFs => "squashfs",
            Format::Vhd => "vhd",
            Format::Vmdk => "vmdk",
            Format::Qcow => "qcow2",
            Format::Apk => "apk",
            Format::Jar => "jar",
            Format::Ipa => "ipa",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Format::SevenZ => "7z",
            Format::Zip => "zip",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarXz => "tar.xz",
            Format::TarBz2 => "tar.bz2",
            Format::TarZst => "tar.zst",
            Format::TarLz => "tar.lz",
            Format::TarLz4 => "tar.lz4",
            Format::TarBr => "tar.br",
            Format::TarLrz => "tar.lrz",
            Format::TarSz => "tar.sz",
            Format::TarZ => "tar.Z",
            Format::TarLzma => "tar.lzma",
            Format::Gz => "gz",
            Format::Xz => "xz",
            Format::Bz2 => "bz2",
            Format::Zst => "zst",
            Format::Lzip => "lzip",
            Format::Lz4 => "lz4",
            Format::Brotli => "brotli",
            Format::Lrzip => "lrzip",
            Format::Snappy => "snappy",
            Format::Rar => "rar",
            Format::Iso => "iso",
            Format::Wim => "wim",
            Format::Dmg => "dmg",
            Format::Exe => "exe",
            Format::Msi => "msi",
            Format::Cab => "cab",
            Format::Lzh => "lzh",
            Format::Arj => "arj",
            Format::Cpio => "cpio",
            Format::Rpm => "rpm",
            Format::Deb => "deb",
            Format::Xar => "xar",
            Format::Ar => "ar",
            Format::Chm => "chm",
            Format::Compress => "Z",
            Format::Lzma => "lzma",
            Format::SquashFs => "squashfs",
            Format::Vhd => "vhd",
            Format::Vmdk => "vmdk",
            Format::Qcow => "qcow",
            Format::Apk => "apk",
            Format::Jar => "jar",
            Format::Ipa => "ipa",
        }
    }

    pub fn from_id(id: &str) -> Option<Format> {
        Some(match id {
            "7z" => Format::SevenZ,
            "zip" => Format::Zip,
            "tar" => Format::Tar,
            "tar.gz" | "tgz" => Format::TarGz,
            "tar.xz" | "txz" => Format::TarXz,
            "tar.bz2" | "tbz2" => Format::TarBz2,
            "tar.zst" | "tzst" => Format::TarZst,
            "tar.lz" => Format::TarLz,
            "tar.lz4" => Format::TarLz4,
            "tar.br" => Format::TarBr,
            "tar.lrz" => Format::TarLrz,
            "tar.sz" => Format::TarSz,
            "tar.Z" | "taz" => Format::TarZ,
            "tar.lzma" => Format::TarLzma,
            "gz" | "gzip" => Format::Gz,
            "xz" => Format::Xz,
            "bz2" | "bzip2" => Format::Bz2,
            "zst" | "zstd" => Format::Zst,
            "lzip" | "lz" => Format::Lzip,
            "lz4" => Format::Lz4,
            "brotli" | "br" => Format::Brotli,
            "lrzip" | "lrz" => Format::Lrzip,
            "snappy" | "sz" => Format::Snappy,
            "rar" => Format::Rar,
            "iso" => Format::Iso,
            "wim" => Format::Wim,
            "dmg" => Format::Dmg,
            "exe" => Format::Exe,
            "msi" => Format::Msi,
            "cab" => Format::Cab,
            "lzh" | "lha" => Format::Lzh,
            "arj" => Format::Arj,
            "cpio" => Format::Cpio,
            "rpm" => Format::Rpm,
            "deb" => Format::Deb,
            "xar" | "pkg" => Format::Xar,
            "ar" => Format::Ar,
            "chm" => Format::Chm,
            "Z" | "compress" => Format::Compress,
            "lzma" => Format::Lzma,
            "squashfs" => Format::SquashFs,
            "vhd" | "vhdx" => Format::Vhd,
            "vmdk" => Format::Vmdk,
            "qcow" | "qcow2" => Format::Qcow,
            "apk" => Format::Apk,
            "jar" => Format::Jar,
            "ipa" => Format::Ipa,
            _ => return None,
        })
    }

    /// The format the pill shows. tar.gz is the gzip row.
    pub fn pill(self) -> Format {
        match self {
            Format::TarGz => Format::Gz,
            Format::TarXz => Format::Xz,
            Format::TarBz2 => Format::Bz2,
            Format::TarZst => Format::Zst,
            Format::TarLz => Format::Lzip,
            Format::TarLz4 => Format::Lz4,
            Format::TarBr => Format::Brotli,
            Format::TarLrz => Format::Lrzip,
            Format::TarSz => Format::Snappy,
            other => other,
        }
    }

    pub fn can_compress(self) -> bool {
        matches!(
            self,
            Format::SevenZ
                | Format::Zip
                | Format::Tar
                | Format::TarGz
                | Format::TarXz
                | Format::TarBz2
                | Format::TarZst
                | Format::TarLz
                | Format::TarLz4
                | Format::TarBr
                | Format::TarLrz
                | Format::TarSz
                | Format::Gz
                | Format::Xz
                | Format::Bz2
                | Format::Zst
                | Format::Lzip
                | Format::Lz4
                | Format::Brotli
                | Format::Lrzip
                | Format::Snappy
                | Format::Wim
                | Format::Dmg
                | Format::Iso
        )
    }

    pub fn is_stream(self) -> bool {
        matches!(
            self,
            Format::Gz
                | Format::Xz
                | Format::Bz2
                | Format::Zst
                | Format::Lzip
                | Format::Lz4
                | Format::Brotli
                | Format::Lrzip
                | Format::Snappy
                | Format::Compress
                | Format::Lzma
        )
    }

    pub fn is_tar_family(self) -> bool {
        matches!(
            self,
            Format::Tar
                | Format::TarGz
                | Format::TarXz
                | Format::TarBz2
                | Format::TarZst
                | Format::TarLz
                | Format::TarLz4
                | Format::TarBr
                | Format::TarLrz
                | Format::TarSz
                | Format::TarZ
                | Format::TarLzma
        )
    }

    /// Compression wrapped around a tar archive, if this is a tar family
    /// member. Plain tar returns `None`.
    pub fn tar_codec(self) -> Option<Format> {
        Some(match self {
            Format::TarGz => Format::Gz,
            Format::TarXz => Format::Xz,
            Format::TarBz2 => Format::Bz2,
            Format::TarZst => Format::Zst,
            Format::TarLz => Format::Lzip,
            Format::TarLz4 => Format::Lz4,
            Format::TarBr => Format::Brotli,
            Format::TarLrz => Format::Lrzip,
            Format::TarSz => Format::Snappy,
            Format::TarZ => Format::Compress,
            Format::TarLzma => Format::Lzma,
            _ => return None,
        })
    }

    pub fn tar_equivalent(self) -> Format {
        match self {
            Format::Gz => Format::TarGz,
            Format::Xz => Format::TarXz,
            Format::Bz2 => Format::TarBz2,
            Format::Zst => Format::TarZst,
            Format::Lzip => Format::TarLz,
            Format::Lz4 => Format::TarLz4,
            Format::Brotli => Format::TarBr,
            Format::Lrzip => Format::TarLrz,
            Format::Snappy => Format::TarSz,
            Format::Compress => Format::TarZ,
            Format::Lzma => Format::TarLzma,
            other => other,
        }
    }

    /// Use the tar wrapper when the selection is a folder or several files.
    pub fn for_inputs(self, inputs: &[impl AsRef<Path>]) -> Format {
        let single_file = inputs.len() == 1 && inputs[0].as_ref().is_file();
        if self.is_stream() && !single_file {
            self.tar_equivalent()
        } else {
            self
        }
    }

    pub fn supports_password(self) -> bool {
        matches!(self.pill(), Format::SevenZ | Format::Zip)
    }

    pub fn supports_split(self) -> bool {
        matches!(self.pill(), Format::SevenZ | Format::Zip)
    }

    pub fn supports_header_encryption(self) -> bool {
        self.pill() == Format::SevenZ
    }

    pub fn supports_aes(self) -> bool {
        self.pill() == Format::Zip
    }

    pub fn supports_level(self) -> bool {
        !matches!(
            self.pill(),
            Format::Tar | Format::Iso | Format::Dmg | Format::Snappy
        )
    }

    /// Plain description of the current stop, or "No compression" when the
    /// format has no level (tar, ISO, DMG, Snappy).
    pub fn method_text(self, level: Level) -> &'static str {
        if self.supports_level() {
            level.description()
        } else {
            "No compression"
        }
    }

    /// Right-hand side of the format menu, and the accelerator.
    pub fn shortcut_label(self) -> Option<&'static str> {
        Some(match self.pill() {
            Format::SevenZ => "Ctrl+Shift+7",
            Format::Zip => "Ctrl+Shift+Z",
            Format::Tar => "Ctrl+Shift+T",
            Format::Zst => "Ctrl+Shift+S",
            Format::Gz => "Ctrl+Shift+G",
            Format::Bz2 => "Ctrl+Shift+B",
            Format::Xz => "Ctrl+Shift+X",
            Format::Lzip => "Ctrl+Shift+L",
            Format::Lz4 => "Ctrl+Shift+4",
            Format::Brotli => "Ctrl+Shift+R",
            Format::Lrzip => "Ctrl+Shift+P",
            Format::Snappy => "Ctrl+Shift+N",
            Format::Wim => "Ctrl+Shift+W",
            Format::Dmg => "Ctrl+Shift+D",
            Format::Iso => "Ctrl+Shift+I",
            _ => return None,
        })
    }

    pub fn shortcut_accel(self) -> Option<&'static str> {
        Some(match self.pill() {
            Format::SevenZ => "<Control><Shift>7",
            Format::Zip => "<Control><Shift>z",
            Format::Tar => "<Control><Shift>t",
            Format::Zst => "<Control><Shift>s",
            Format::Gz => "<Control><Shift>g",
            Format::Bz2 => "<Control><Shift>b",
            Format::Xz => "<Control><Shift>x",
            Format::Lzip => "<Control><Shift>l",
            Format::Lz4 => "<Control><Shift>4",
            Format::Brotli => "<Control><Shift>r",
            Format::Lrzip => "<Control><Shift>p",
            Format::Snappy => "<Control><Shift>n",
            Format::Wim => "<Control><Shift>w",
            Format::Dmg => "<Control><Shift>d",
            Format::Iso => "<Control><Shift>i",
            _ => return None,
        })
    }

    pub fn from_name(name: &str) -> Option<Format> {
        let lower = name.to_lowercase();
        if let Some(format) = split_volume(&lower) {
            return Some(format);
        }
        const SUFFIXES: &[(&str, Format)] = &[
            (".tar.lzma", Format::TarLzma),
            (".tar.zstd", Format::TarZst),
            (".tar.lz4", Format::TarLz4),
            (".tar.lrz", Format::TarLrz),
            (".tar.bz2", Format::TarBz2),
            (".tar.zst", Format::TarZst),
            (".tar.gz", Format::TarGz),
            (".tar.xz", Format::TarXz),
            (".tar.lz", Format::TarLz),
            (".tar.br", Format::TarBr),
            (".tar.sz", Format::TarSz),
            (".tar.z", Format::TarZ),
            (".squashfs", Format::SquashFs),
            (".tbz2", Format::TarBz2),
            (".tzst", Format::TarZst),
            (".tlz4", Format::TarLz4),
            (".tgz", Format::TarGz),
            (".tbz", Format::TarBz2),
            (".tb2", Format::TarBz2),
            (".txz", Format::TarXz),
            (".taz", Format::TarZ),
            (".lzma", Format::Lzma),
            (".zstd", Format::Zst),
            (".lz4", Format::Lz4),
            (".lrz", Format::Lrzip),
            (".bz2", Format::Bz2),
            (".zst", Format::Zst),
            (".tar", Format::Tar),
            (".zip", Format::Zip),
            (".rar", Format::Rar),
            (".iso", Format::Iso),
            (".7z", Format::SevenZ),
            (".apk", Format::Apk),
            (".jar", Format::Jar),
            (".ipa", Format::Ipa),
            (".wim", Format::Wim),
            (".swm", Format::Wim),
            (".esd", Format::Wim),
            (".dmg", Format::Dmg),
            (".exe", Format::Exe),
            (".msi", Format::Msi),
            (".msp", Format::Msi),
            (".cab", Format::Cab),
            (".lzh", Format::Lzh),
            (".lha", Format::Lzh),
            (".arj", Format::Arj),
            (".cpio", Format::Cpio),
            (".rpm", Format::Rpm),
            (".deb", Format::Deb),
            (".udeb", Format::Deb),
            (".xar", Format::Xar),
            (".pkg", Format::Xar),
            (".chm", Format::Chm),
            (".qcow2", Format::Qcow),
            (".qcow", Format::Qcow),
            (".vmdk", Format::Vmdk),
            (".vhdx", Format::Vhd),
            (".vhd", Format::Vhd),
            (".br", Format::Brotli),
            (".sz", Format::Snappy),
            (".lz", Format::Lzip),
            (".gz", Format::Gz),
            (".xz", Format::Xz),
            (".z", Format::Compress),
        ];
        SUFFIXES
            .iter()
            .find(|(suffix, _)| lower.ends_with(suffix))
            .map(|(_, format)| *format)
            .or_else(|| {
                // `.a` is a short suffix and matches names like `Makefile.a`
                // only when it really is the whole extension.
                if lower.ends_with(".a") {
                    Some(Format::Ar)
                } else {
                    None
                }
            })
    }

    /// Whether this format can be created with the tools found on `PATH`.
    pub fn create_ready(self, tools: &Tools) -> bool {
        match self.pill() {
            Format::SevenZ | Format::Wim => tools.seven_zip.is_some(),
            Format::Zip => tools.seven_zip.is_some() || tools.zip.is_some(),
            Format::Tar | Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Snappy => {
                true
            }
            Format::Lzip => tools.lzip.is_some(),
            Format::Lz4 => tools.lz4.is_some(),
            Format::Brotli => tools.brotli.is_some(),
            Format::Lrzip => tools.lrzip.is_some(),
            Format::Iso => tools.iso_creator.is_some(),
            Format::Dmg => tools.dmg_build.is_some(),
            _ => false,
        }
    }

    pub fn create_hint(self) -> &'static str {
        match self.pill() {
            Format::SevenZ => "Install 7z to create 7Z archives.",
            Format::Zip => "Install 7z to create ZIP archives.",
            Format::Wim => "Install 7z to create WIM archives.",
            Format::Lzip => "Install lzip to create LZIP archives.",
            Format::Lz4 => "Install lz4 to create LZ4 archives.",
            Format::Brotli => "Install brotli to create Brotli archives.",
            Format::Lrzip => "Install lrzip to create LRZIP archives.",
            Format::Iso => "Install xorriso to create ISO images.",
            Format::Dmg => "Install dmg (libdmg-hfsplus) to create DMG images.",
            _ => "This format cannot be created.",
        }
    }

    /// Program the user should install before this format can be extracted,
    /// when nothing that can read it is present.
    pub fn extract_tool(self) -> &'static str {
        match self {
            Format::Rar => "unar",
            Format::Lzip | Format::TarLz => "lzip",
            Format::Lz4 | Format::TarLz4 => "lz4",
            Format::Brotli | Format::TarBr => "brotli",
            Format::Lrzip | Format::TarLrz => "lrzip",
            Format::Cab => "cabextract",
            Format::Exe => "7z",
            _ => "7z",
        }
    }
}

/// Rows for the format menu. DMG is left out unless a free creator exists.
pub fn create_choices() -> Vec<CreateChoice> {
    let tools = tools::tools();
    let mut formats: Vec<Format> = CREATE_FORMATS.to_vec();
    if tools.dmg_build.is_some() {
        // DMG sits between WIM and ISO, matching the usual order.
        if let Some(iso) = formats.iter().position(|format| *format == Format::Iso) {
            formats.insert(iso, Format::Dmg);
        } else {
            formats.push(Format::Dmg);
        }
    }
    formats
        .into_iter()
        .map(|format| CreateChoice {
            format,
            available: format.create_ready(tools),
            hint: format.create_hint(),
        })
        .collect()
}

fn split_volume(lower: &str) -> Option<Format> {
    let (rest, num) = lower.rsplit_once('.')?;
    if num.len() == 3 && num.chars().all(|c| c.is_ascii_digit()) {
        if rest.ends_with(".7z") {
            return Some(Format::SevenZ);
        }
        if rest.ends_with(".zip") {
            return Some(Format::Zip);
        }
        if rest.ends_with(".wim") {
            return Some(Format::Wim);
        }
    }
    None
}

/// Extension first, then magic bytes. A file with no recognised name can
/// still be opened: extraction tries 7-Zip before it gives up.
pub fn detect(path: &Path) -> Option<Format> {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if let Some(format) = Format::from_name(name) {
            return Some(format);
        }
    }
    detect_magic(path)
}

fn detect_magic(path: &Path) -> Option<Format> {
    let prefix = read_at(path, 0, 64)?;
    if prefix.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return Some(Format::SevenZ);
    }
    if prefix.starts_with(b"PK\x03\x04")
        || prefix.starts_with(b"PK\x05\x06")
        || prefix.starts_with(b"PK\x07\x08")
    {
        return Some(Format::Zip);
    }
    if prefix.starts_with(b"Rar!\x1a\x07") {
        return Some(Format::Rar);
    }
    if prefix.starts_with(&[0x1F, 0x8B]) {
        return Some(Format::Gz);
    }
    if prefix.starts_with(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]) {
        return Some(Format::Xz);
    }
    if prefix.starts_with(b"BZh") {
        return Some(Format::Bz2);
    }
    if prefix.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return Some(Format::Zst);
    }
    if prefix.starts_with(&[0x04, 0x22, 0x4D, 0x18]) {
        return Some(Format::Lz4);
    }
    if prefix.starts_with(b"LZIP") {
        return Some(Format::Lzip);
    }
    if prefix.starts_with(b"LRZI") {
        return Some(Format::Lrzip);
    }
    if prefix.starts_with(&[0xFF, 0x06, 0x00, 0x00, b's', b'N', b'a', b'P']) {
        return Some(Format::Snappy);
    }
    if prefix.starts_with(&[0x1F, 0x9D]) {
        return Some(Format::Compress);
    }
    if prefix.starts_with(b"MSCF") {
        return Some(Format::Cab);
    }
    if prefix.starts_with(b"MSWIM\0\0\0") {
        return Some(Format::Wim);
    }
    if prefix.starts_with(b"ITSF") {
        return Some(Format::Chm);
    }
    if prefix.starts_with(&[0x60, 0xEA]) {
        return Some(Format::Arj);
    }
    if prefix.starts_with(b"!<arch>\n") {
        return Some(Format::Ar);
    }
    if prefix.starts_with(b"xar!") {
        return Some(Format::Xar);
    }
    if prefix.starts_with(&[0xED, 0xAB, 0xEE, 0xDB]) {
        return Some(Format::Rpm);
    }
    if prefix.starts_with(b"070701") || prefix.starts_with(b"070702") || prefix.starts_with(b"070707")
    {
        return Some(Format::Cpio);
    }
    if prefix.starts_with(b"QFI\xfb") {
        return Some(Format::Qcow);
    }
    if prefix.starts_with(b"KDMV") {
        return Some(Format::Vmdk);
    }
    if prefix.starts_with(b"conectix") {
        return Some(Format::Vhd);
    }
    if prefix.starts_with(b"hsqs") || prefix.starts_with(b"sqsh") {
        return Some(Format::SquashFs);
    }
    if prefix.len() > 8 && &prefix[2..5] == b"-lh" {
        return Some(Format::Lzh);
    }
    if prefix.starts_with(b"MZ") {
        return Some(Format::Exe);
    }
    if let Some(block) = read_at(path, 257, 8) {
        if block.starts_with(b"ustar") {
            return Some(Format::Tar);
        }
    }
    if let Some(cd) = read_at(path, 0x8001, 5) {
        if cd.starts_with(b"CD001") {
            return Some(Format::Iso);
        }
    }
    if let Some(apfs) = read_at(path, 32, 4) {
        if apfs.starts_with(b"NXSB") {
            return Some(Format::Dmg);
        }
    }
    if let Some(hfs) = read_at(path, 1024, 2) {
        if hfs.starts_with(b"H+") || hfs.starts_with(b"HX") || hfs.starts_with(b"BD") {
            return Some(Format::Dmg);
        }
    }
    if let Some(len) = std::fs::metadata(path).ok().map(|meta| meta.len()) {
        if len > 512 {
            if let Some(tail) = read_at(path, len - 512, 8) {
                if tail.starts_with(b"koly") {
                    return Some(Format::Dmg);
                }
                if tail.starts_with(b"conectix") {
                    return Some(Format::Vhd);
                }
            }
        }
    }
    None
}

fn read_at(path: &Path, offset: u64, len: usize) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

/// True when every path looks like an archive this app can extract.
pub fn all_archives(paths: &[impl AsRef<Path>]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| detect(path.as_ref()).is_some())
}

/// A split size the menu can drop into the field. `label` is what the field
/// shows, and [`parse_size`] turns that label back into `bytes`.
///
/// Media sizes are the published user capacity, or a binary size that sits
/// under that capacity when the disc's sector count is larger:
///
/// | Preset | Bytes | Why |
/// | --- | --- | --- |
/// | 1.44 MB Floppy disk | 1,457,664 | 80 tracks × 18 sectors × 512 bytes × 2 sides |
/// | 650 MB CD | 681,574,400 | 650 × 1024², under a 333,000-sector CD (681,984,000) |
/// | 700 MB CD | 734,003,200 | 700 × 1024², under an 80-minute CD (737,280,000) |
/// | 4.7 GB DVD-5 | 4,700,372,992 | DVD-5 user data |
/// | 8.5 GB DVD-9 | 8,543,666,176 | DVD-9 user data |
/// | 25 GB BD-25 | 25,025,314,816 | 12,219,392 sectors × 2048 |
/// | 50 GB BD-50 | 50,050,629,632 | 24,438,784 sectors × 2048 |
/// | 100 GB BD-XL | 100,103,356,416 | 48,878,592 sectors × 2048 |
/// | 4 GB FAT32 | 4,294,967,295 | Largest file a FAT32 volume can store (2³² − 1) |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitPreset {
    pub label: &'static str,
    pub bytes: u64,
}

pub const SPLIT_PRESETS: &[SplitPreset] = &[
    SplitPreset {
        label: "1.44 MB Floppy disk",
        bytes: 1_457_664,
    },
    SplitPreset {
        label: "650 MB CD",
        bytes: 650 * 1024 * 1024,
    },
    SplitPreset {
        label: "700 MB CD",
        bytes: 700 * 1024 * 1024,
    },
    SplitPreset {
        label: "4.7 GB DVD-5",
        bytes: 4_700_372_992,
    },
    SplitPreset {
        label: "8.5 GB DVD-9",
        bytes: 8_543_666_176,
    },
    SplitPreset {
        label: "25 GB BD-25",
        bytes: 25_025_314_816,
    },
    SplitPreset {
        label: "50 GB BD-50",
        bytes: 50_050_629_632,
    },
    SplitPreset {
        label: "100 GB BD-XL",
        bytes: 100_103_356_416,
    },
    SplitPreset {
        label: "4 GB FAT32",
        bytes: 4_294_967_295,
    },
];

/// `5 MB`, `700MB`, `1.5 GB`, `512K`, or a bare byte count.
/// An empty string is no split. A preset label uses that preset's exact size.
pub fn parse_size(text: &str) -> Option<Option<u64>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(None);
    }
    let lower_words = text.to_ascii_lowercase();
    if let Some(preset) = SPLIT_PRESETS
        .iter()
        .find(|preset| preset.label.eq_ignore_ascii_case(text))
    {
        return Some(Some(preset.bytes));
    }
    let lower = lower_words.replace(' ', "");
    if lower == "dvd" {
        return Some(Some(4_700_372_992));
    }
    let unit_at = lower
        .find(|c: char| c != '.' && !c.is_ascii_digit())
        .unwrap_or(lower.len());
    if unit_at == 0 {
        return None;
    }
    let number: f64 = lower[..unit_at].parse().ok()?;
    if !(number.is_finite() && number > 0.0) {
        return None;
    }
    let mul: f64 = match &lower[unit_at..] {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = (number * mul).round() as u64;
    if bytes == 0 {
        None
    } else {
        Some(Some(bytes))
    }
}

/// How hard the compressor should work. Six stops, evenly spaced.
/// The window prints Store, Fast, Normal, and Slow under stops 1, 2, 4, and 6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Store,
    Fastest,
    Fast,
    Normal,
    High,
    Slowest,
}

impl Level {
    /// Tick caption. Stops 3 and 5 are dots, so they have no caption.
    pub fn label(self) -> &'static str {
        match self {
            Level::Store => "Store",
            Level::Fastest => "Fast",
            Level::Fast | Level::High => "",
            Level::Normal => "Normal",
            Level::Slowest => "Slow",
        }
    }

    /// The sentence next to "Method:".
    pub fn description(self) -> &'static str {
        match self {
            Level::Store => "No compression",
            Level::Fastest => "Fastest, less compression",
            Level::Fast => "Fast, less compression",
            Level::Normal => "Normal",
            Level::High => "Slow, more compression",
            Level::Slowest => "Slowest, more compression",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Level::Store => "store",
            Level::Fastest => "fastest",
            Level::Fast => "faster",
            Level::Normal => "normal",
            Level::High => "slower",
            Level::Slowest => "slowest",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "store" => Level::Store,
            // The old four-stop "fast" was the stop labeled Fast.
            "fast" | "fastest" => Level::Fastest,
            "faster" => Level::Fast,
            "slower" => Level::High,
            // The old four-stop "slow" and "maximum" were the last stop.
            "slow" | "slowest" | "maximum" => Level::Slowest,
            _ => Level::Normal,
        }
    }

    pub const STOPS: [Level; 6] = [
        Level::Store,
        Level::Fastest,
        Level::Fast,
        Level::Normal,
        Level::High,
        Level::Slowest,
    ];

    pub fn index(self) -> f64 {
        match self {
            Level::Store => 0.0,
            Level::Fastest => 1.0,
            Level::Fast => 2.0,
            Level::Normal => 3.0,
            Level::High => 4.0,
            Level::Slowest => 5.0,
        }
    }

    pub fn from_index(index: f64) -> Self {
        match index.round().clamp(0.0, 5.0) as i32 {
            0 => Level::Store,
            1 => Level::Fastest,
            2 => Level::Fast,
            4 => Level::High,
            5 => Level::Slowest,
            _ => Level::Normal,
        }
    }

    pub(crate) fn gzip(self) -> u32 {
        match self {
            Level::Store => 0,
            Level::Fastest => 1,
            Level::Fast => 3,
            Level::Normal => 6,
            Level::High => 8,
            Level::Slowest => 9,
        }
    }

    pub(crate) fn xz(self) -> u32 {
        match self {
            Level::Store => 0,
            Level::Fastest => 1,
            Level::Fast => 3,
            Level::Normal => 6,
            Level::High => 8,
            Level::Slowest => 9,
        }
    }

    pub(crate) fn bzip2(self) -> u32 {
        match self {
            Level::Store | Level::Fastest => 1,
            Level::Fast => 3,
            Level::Normal => 6,
            Level::High => 8,
            Level::Slowest => 9,
        }
    }

    pub(crate) fn zstd(self) -> i32 {
        match self {
            Level::Store | Level::Fastest => 1,
            Level::Fast => 3,
            Level::Normal => 6,
            Level::High => 12,
            Level::Slowest => 19,
        }
    }

    pub(crate) fn seven(self) -> u8 {
        match self {
            Level::Store => 0,
            Level::Fastest => 1,
            Level::Fast => 3,
            Level::Normal => 5,
            Level::High => 7,
            Level::Slowest => 9,
        }
    }

    pub(crate) fn zip_cli(self) -> &'static str {
        match self {
            Level::Store => "-0",
            Level::Fastest => "-1",
            Level::Fast => "-3",
            Level::Normal => "-6",
            Level::High => "-8",
            Level::Slowest => "-9",
        }
    }

    pub(crate) fn lz4(self) -> &'static str {
        match self {
            Level::Store | Level::Fastest => "-1",
            Level::Fast => "-3",
            Level::Normal => "-6",
            Level::High => "-8",
            Level::Slowest => "-9",
        }
    }

    pub(crate) fn brotli(self) -> &'static str {
        match self {
            Level::Store => "0",
            Level::Fastest => "1",
            Level::Fast => "3",
            Level::Normal => "6",
            Level::High => "9",
            Level::Slowest => "11",
        }
    }

    pub(crate) fn lzip(self) -> &'static str {
        match self {
            Level::Store => "-0",
            Level::Fastest => "-1",
            Level::Fast => "-3",
            Level::Normal => "-6",
            Level::High => "-8",
            Level::Slowest => "-9",
        }
    }

    pub(crate) fn lrzip_level(self) -> &'static str {
        match self {
            Level::Store | Level::Fastest => "1",
            Level::Fast => "3",
            Level::Normal => "5",
            Level::High => "7",
            Level::Slowest => "9",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes_prefer_the_compound_extension() {
        assert_eq!(Format::from_name("Photos.tar.gz"), Some(Format::TarGz));
        assert_eq!(Format::from_name("Photos.tgz"), Some(Format::TarGz));
        assert_eq!(Format::from_name("Photos.tar.zst"), Some(Format::TarZst));
        assert_eq!(Format::from_name("Photos.tar.lz4"), Some(Format::TarLz4));
        assert_eq!(Format::from_name("Photos.tar.lz"), Some(Format::TarLz));
        assert_eq!(Format::from_name("Photos.tar.br"), Some(Format::TarBr));
        assert_eq!(Format::from_name("notes.gz"), Some(Format::Gz));
        assert_eq!(Format::from_name("disk.iso"), Some(Format::Iso));
        assert_eq!(Format::from_name("book.rar"), Some(Format::Rar));
        assert_eq!(Format::from_name("app.apk"), Some(Format::Apk));
        assert_eq!(Format::from_name("setup.exe"), Some(Format::Exe));
        assert_eq!(Format::from_name("lib.a"), Some(Format::Ar));
        assert_eq!(Format::from_name("vol.7z.001"), Some(Format::SevenZ));
        assert_eq!(Format::from_name("Makefile"), None);
    }

    #[test]
    fn ids_round_trip() {
        for format in CREATE_FORMATS.iter().copied().chain([
            Format::Rar,
            Format::Dmg,
            Format::Exe,
            Format::Cab,
            Format::Deb,
            Format::TarGz,
            Format::TarLz4,
        ]) {
            assert_eq!(Format::from_id(format.id()), Some(format));
        }
    }

    #[test]
    fn sizes_accept_the_example() {
        assert_eq!(parse_size(""), Some(None));
        assert_eq!(parse_size("   "), Some(None));
        assert_eq!(parse_size("5 MB"), Some(Some(5 * 1024 * 1024)));
        assert_eq!(parse_size("700MB"), Some(Some(700 * 1024 * 1024)));
        assert_eq!(parse_size("1 GB"), Some(Some(1024 * 1024 * 1024)));
        assert_eq!(
            parse_size("1.5 GB"),
            Some(Some((1.5_f64 * 1024.0 * 1024.0 * 1024.0).round() as u64))
        );
        assert_eq!(parse_size("512 KB"), Some(Some(512 * 1024)));
        assert_eq!(parse_size("4096"), Some(Some(4096)));
        assert_eq!(parse_size("100M"), Some(Some(100 * 1024 * 1024)));
        assert_eq!(
            parse_size("4.7 GB"),
            Some(Some((4.7_f64 * 1024.0 * 1024.0 * 1024.0).round() as u64))
        );
        assert_eq!(parse_size("nope"), None);
        for preset in SPLIT_PRESETS {
            assert_eq!(parse_size(preset.label), Some(Some(preset.bytes)));
        }
        assert_eq!(parse_size("4.7 GB DVD-5"), Some(Some(4_700_372_992)));
        assert_eq!(parse_size("4 GB FAT32"), Some(Some(4_294_967_295)));
        assert_ne!(parse_size("4 GB"), parse_size("4 GB FAT32"));
    }

    #[test]
    fn magic_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        assert_eq!(
            detect(&write("noext", &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C])),
            Some(Format::SevenZ)
        );
        assert_eq!(
            detect(&write("mystery", b"Rar!\x1a\x07\x00")),
            Some(Format::Rar)
        );
        assert_eq!(detect(&write("cab", b"MSCF\0\0\0\0")), Some(Format::Cab));
        assert_eq!(detect(&write("z", &[0x1F, 0x9D, 0x90])), Some(Format::Compress));
        assert_eq!(detect(&write("lz", b"LZIP\x01")), Some(Format::Lzip));
        assert_eq!(
            detect(&write("frame", &[0x04, 0x22, 0x4D, 0x18])),
            Some(Format::Lz4)
        );
        assert_eq!(detect(&write("disk", b"QFI\xfb\0\0\0")), Some(Format::Qcow));
        assert_eq!(detect(&write("app", b"MZ\x90\0")), Some(Format::Exe));
    }

    #[test]
    fn old_ids_land_on_the_labeled_stops() {
        assert_eq!(Level::from_id("fast"), Level::Fastest);
        assert_eq!(Level::from_id("fast").label(), "Fast");
        assert_eq!(Level::from_id("fast").description(), "Fastest, less compression");
        assert_eq!(Level::from_id("maximum"), Level::Slowest);
        assert_eq!(Level::from_id("slow"), Level::Slowest);
        assert_eq!(Level::from_id("slow").label(), "Slow");
        assert_eq!(Level::STOPS.map(|level| level.seven()), [0, 1, 3, 5, 7, 9]);
        let captions: Vec<_> = Level::STOPS
            .into_iter()
            .map(Level::label)
            .filter(|label| !label.is_empty())
            .collect();
        assert_eq!(captions, ["Store", "Fast", "Normal", "Slow"]);
    }
}
