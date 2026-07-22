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

if [[ "${SAGE_INSTALL_SERVICE:-0}" == "1" ]]; then
  mkdir -p "$UNIT_DIR"
  cat > "$UNIT_DIR/sage.service" <<EOF
[Unit]
Description=Sage voice-to-text daemon
After=default.target

[Service]
Type=simple
ExecStart=$INSTALL_DIR/sage daemon
Restart=on-failure

[Install]
WantedBy=default.target
EOF
  systemctl --user daemon-reload || true
  systemctl --user enable --now sage.service || true
  echo "systemd user unit: sage.service enabled"
fi

echo "next:"
echo "  sage model pull"
echo "  sage doctor"
echo "  sage start / stop"
