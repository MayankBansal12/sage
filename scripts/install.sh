#!/usr/bin/env bash
# Sage installer — copies release binary to ~/.local/bin and optional systemd user unit.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_SRC="${1:-$REPO_ROOT/target/release/sage}"
INSTALL_DIR="${SAGE_INSTALL_DIR:-$HOME/.local/bin}"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

if [[ ! -x "$BIN_SRC" ]]; then
  echo "error: binary not found or not executable: $BIN_SRC" >&2
  echo "build first: cargo build --release" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
install -m 755 "$BIN_SRC" "$INSTALL_DIR/sage"
echo "installed: $INSTALL_DIR/sage"

# Ensure PATH hint
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "note: add $INSTALL_DIR to PATH if needed" ;;
esac

if [[ "${SAGE_INSTALL_SERVICE:-1}" == "1" ]]; then
  mkdir -p "$UNIT_DIR"
  cat > "$UNIT_DIR/sage.service" <<EOF
[Unit]
Description=Sage voice-to-text daemon
After=default.target

[Service]
Type=simple
ExecStart="$INSTALL_DIR/sage" daemon
Restart=on-failure
RestartSec=1

[Install]
WantedBy=default.target
EOF
  if systemctl --user daemon-reload && systemctl --user enable --now sage.service; then
    echo "systemd user unit: sage.service enabled"
  else
    echo "warning: could not enable sage.service; use a desktop shortcut that runs 'sage toggle'" >&2
  fi
fi

echo "next:"
echo "  sage model pull"
echo "  sage doctor"
echo "  sage start / stop"
