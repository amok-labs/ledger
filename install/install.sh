#!/usr/bin/env bash
# Ledger daemon installer for macOS
# Builds ledgerd, installs as a launchd service with auto-restart.
#
# Usage: ./install/install.sh [--uninstall]
set -euo pipefail

LEDGER_DIR="$HOME/.ledger"
BIN_DIR="$LEDGER_DIR/bin"
PLIST_NAME="com.ledger.daemon"
PLIST_DEST="$HOME/Library/LaunchAgents/$PLIST_NAME.plist"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info()  { echo -e "${GREEN}[info]${NC} $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC} $*"; }
error() { echo -e "${RED}[error]${NC} $*" >&2; }

uninstall() {
    info "Uninstalling ledger daemon..."

    if [ -f "$PLIST_DEST" ]; then
        launchctl unload "$PLIST_DEST" 2>/dev/null || true
        rm -f "$PLIST_DEST"
        info "Removed launchd plist"
    fi

    if [ -S "$LEDGER_DIR/ledger.sock" ]; then
        rm -f "$LEDGER_DIR/ledger.sock"
        info "Removed socket"
    fi

    info "Uninstall complete. Data preserved at $LEDGER_DIR"
    info "To remove all data: rm -rf $LEDGER_DIR"
    exit 0
}

# Handle --uninstall flag
if [ "${1:-}" = "--uninstall" ]; then
    uninstall
fi

# Check prerequisites
if ! command -v cargo &>/dev/null; then
    error "cargo not found. Install Rust: https://rustup.rs"
    exit 1
fi

info "Installing ledger daemon..."

# Create directories
mkdir -p "$BIN_DIR"
mkdir -p "$HOME/Library/LaunchAgents"
info "Created $LEDGER_DIR"

# Build release binary
info "Building ledgerd (release mode)..."
cd "$REPO_DIR"
cargo build --release -p ledgerd

# Copy binary
BINARY_SRC="$REPO_DIR/target/release/ledgerd"
if [ ! -f "$BINARY_SRC" ]; then
    error "Build succeeded but binary not found at $BINARY_SRC"
    exit 1
fi

cp "$BINARY_SRC" "$BIN_DIR/ledgerd"
chmod 755 "$BIN_DIR/ledgerd"
info "Installed binary to $BIN_DIR/ledgerd"

# Unload existing service if present
if [ -f "$PLIST_DEST" ]; then
    warn "Unloading existing service..."
    launchctl unload "$PLIST_DEST" 2>/dev/null || true
fi

# Generate plist from template
sed -e "s|__LEDGERD_BIN__|$BIN_DIR/ledgerd|g" \
    -e "s|__LEDGER_DIR__|$LEDGER_DIR|g" \
    -e "s|__HOME__|$HOME|g" \
    "$SCRIPT_DIR/com.ledger.daemon.plist" > "$PLIST_DEST"
info "Installed plist to $PLIST_DEST"

# Load service
launchctl load "$PLIST_DEST"
info "Service loaded"

# Wait for socket
sleep 2
if [ -S "$LEDGER_DIR/ledger.sock" ]; then
    info "Daemon is running (socket at $LEDGER_DIR/ledger.sock)"
else
    warn "Socket not found yet. Check logs:"
    warn "  stdout: $LEDGER_DIR/ledgerd.stdout.log"
    warn "  stderr: $LEDGER_DIR/ledgerd.stderr.log"
fi

info "Installation complete!"
info ""
info "Usage:"
info "  ledger log --source test --type ping --payload '{}'"
info "  ledger query --type ping"
info "  ledger status"
info "  ledger subscribe"
info ""
info "To uninstall: $SCRIPT_DIR/install.sh --uninstall"
