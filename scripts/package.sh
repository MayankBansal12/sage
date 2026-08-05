#!/usr/bin/env bash
# Build a portable release tarball (phase 10). AppImage optional if appimagetool exists.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
source "$HOME/.cargo/env" 2>/dev/null || true

cargo build --release
VERSION="$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)"
STAGE="dist/sage-${VERSION}-linux-x86_64"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp target/release/sage "$STAGE/"
cp README.md MODEL_LICENSES.md PHASES.md scripts/install.sh "$STAGE/" 2>/dev/null || true
chmod +x "$STAGE/install.sh" "$STAGE/sage"

mkdir -p dist
TAR="dist/sage-${VERSION}-linux-x86_64.tar.gz"
tar -C dist -czf "sage-${VERSION}-linux-x86_64.tar.gz" "sage-${VERSION}-linux-x86_64"
mv "sage-${VERSION}-linux-x86_64.tar.gz" "$TAR"
echo "created $TAR"

if command -v appimagetool >/dev/null 2>&1; then
  echo "appimagetool found — AppImage packaging can be added later"
else
  echo "note: appimagetool not installed; shipping tarball only"
fi
