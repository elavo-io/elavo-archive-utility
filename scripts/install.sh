#!/bin/sh
# Install Archive Utility for the current user, or system-wide when PREFIX is set.
#   ./scripts/install.sh
#   PREFIX=/usr/local ./scripts/install.sh
# The image build runs `cargo build --release --locked` itself, then
# `DESTDIR=/out scripts/install`.
set -eu
cd "$(dirname "$0")/.."
prefix="${PREFIX:-$HOME/.local}"

cargo build --release
PREFIX="$prefix" ./scripts/install

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" || true
fi
if command -v gtk4-update-icon-cache >/dev/null 2>&1; then
    gtk4-update-icon-cache -q "$prefix/share/icons/hicolor" || true
fi

echo "Installed elavo-archive-utility to $prefix/bin/elavo-archive-utility"
