#!/bin/sh
# Install Archive Utility for the current user, or system-wide when PREFIX is set.
#   ./scripts/install.sh
#   PREFIX=/usr/local ./scripts/install.sh
set -eu
cd "$(dirname "$0")/.."
prefix="${PREFIX:-$HOME/.local}"

cargo build --release
install -Dm755 target/release/elavo-archive-utility "$prefix/bin/elavo-archive-utility"
install -Dm644 data/io.elavo.ArchiveUtility.desktop \
    "$prefix/share/applications/io.elavo.ArchiveUtility.desktop"
install -Dm644 data/io.elavo.ArchiveUtility.metainfo.xml \
    "$prefix/share/metainfo/io.elavo.ArchiveUtility.metainfo.xml"
install -Dm644 data/icons/hicolor/scalable/apps/io.elavo.ArchiveUtility.svg \
    "$prefix/share/icons/hicolor/scalable/apps/io.elavo.ArchiveUtility.svg"
install -Dm644 data/icons/hicolor/symbolic/apps/io.elavo.ArchiveUtility-symbolic.svg \
    "$prefix/share/icons/hicolor/symbolic/apps/io.elavo.ArchiveUtility-symbolic.svg"
install -Dm644 data/icons/hicolor/128x128/apps/io.elavo.ArchiveUtility.png \
    "$prefix/share/icons/hicolor/128x128/apps/io.elavo.ArchiveUtility.png"
install -Dm644 data/icons/hicolor/256x256/apps/io.elavo.ArchiveUtility.png \
    "$prefix/share/icons/hicolor/256x256/apps/io.elavo.ArchiveUtility.png"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" || true
fi
if command -v gtk4-update-icon-cache >/dev/null 2>&1; then
    gtk4-update-icon-cache -q "$prefix/share/icons/hicolor" || true
fi

echo "Installed elavo-archive-utility to $prefix/bin/elavo-archive-utility"
