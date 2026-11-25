#!/usr/bin/env bash
# 🔱 Atlantis Turnkey Installer
# Usage:
#   curl -sSL https://raw.githubusercontent.com/ziuus/atlantis/master/install.sh | bash
#   ./install.sh [OPTIONS]
set -e

PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
CONFIG_DIR="$HOME/.config/atlantis"
AUTOSTART_DIR="$HOME/.config/autostart"
SYSTEMD_USER_DIR="$HOME/.config/systemd/user"

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -d "$REPO_DIR/.git" ] || [ -f "$REPO_DIR/Cargo.toml" ]; then
    cd "$REPO_DIR"
    LOCAL_BUILD=true
else
    LOCAL_BUILD=false
fi

DRY_RUN=false
CHECK_ONLY=false
INSTALL_SYSTEMD=true
INSTALL_AUTOSTART=false
DO_UNINSTALL=false
INSTALL_DEPS=false

show_help() {
    cat << EOF
🔱 Atlantis — Turnkey Wayland Desktop Environment Installer

Usage: $(basename "$0") [OPTIONS]

Options:
  --deps         Detect and install system dependencies via package manager
  --check        Check all required prerequisites and exit without compiling
  --dry-run      Simulate installation, display planned actions without modifying system
  --no-systemd   Do not install/enable systemd user service
  --autostart    Install XDG autostart desktop entry (atlantis.desktop)
  --uninstall    Remove installed binaries, systemd service, and autostart entries
  -h, --help     Show this help message and exit

Environment Variables:
  PREFIX         Installation prefix (default: \$HOME/.local)
EOF
}

for arg in "$@"; do
    case "$arg" in
        --deps)
            INSTALL_DEPS=true
            ;;
        --check)
            CHECK_ONLY=true
            ;;
        --dry-run)
            DRY_RUN=true
            ;;
        --no-systemd)
            INSTALL_SYSTEMD=false
            ;;
        --autostart)
            INSTALL_AUTOSTART=true
            ;;
        --uninstall|uninstall)
            DO_UNINSTALL=true
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo "ERROR: Unknown option '$arg'" >&2
            echo "Run '$(basename "$0") --help' for usage." >&2
            exit 1
            ;;
    esac
done

if [ "$DO_UNINSTALL" = true ]; then
    echo "==> Uninstalling Atlantis desktop environment..."
    killall -9 atlantis liquid-shell 2>/dev/null || true
    systemctl --user stop atlantis.service liquid-shell.service 2>/dev/null || true
    systemctl --user disable atlantis.service liquid-shell.service 2>/dev/null || true
    rm -f "$SYSTEMD_USER_DIR/atlantis.service" "$SYSTEMD_USER_DIR/liquid-shell.service"
    rm -f "$BIN_DIR/atlantis" "$BIN_DIR/liquid-shell"
    rm -f "$BIN_DIR/atlantis-ctl" "$BIN_DIR/island-ctl"
    rm -f "$AUTOSTART_DIR/atlantis.desktop" "$AUTOSTART_DIR/liquid-shell.desktop"
    echo "==> Removed binaries, systemd services, and autostart entries."
    echo "==> Configurations in $CONFIG_DIR preserved."
    exit 0
fi

# Detect package manager and install dependencies if requested
install_dependencies() {
    echo "==> Detecting system package manager for dependencies..."
    if command -v pacman &>/dev/null; then
        echo "==> Installing dependencies via pacman (Arch/Manjaro)..."
        sudo pacman -S --needed --noconfirm rust wayland wayland-protocols libxkbcommon playerctl wireplumber brightnessctl btop
    elif command -v dnf &>/dev/null; then
        echo "==> Installing dependencies via dnf (Fedora)..."
        sudo dnf install -y rust cargo wayland-devel wayland-protocols-devel libxkbcommon-devel playerctl wireplumber brightnessctl btop
    elif command -v apt-get &>/dev/null; then
        echo "==> Installing dependencies via apt (Debian/Ubuntu)..."
        sudo apt-get update
        sudo apt-get install -y rustc cargo libwayland-dev wayland-protocols libxkbcommon-dev playerctl wireplumber brightnessctl btop
    elif command -v xbps-install &>/dev/null; then
        echo "==> Installing dependencies via xbps (Void)..."
        sudo xbps-install -Sy rust cargo wayland-devel wayland-protocols libxkbcommon-devel playerctl wireplumber brightnessctl btop
    else
        echo "==> Package manager not recognized. Please install manually:"
        echo "    Rust (cargo), wayland, wayland-protocols, libxkbcommon, playerctl, wireplumber, brightnessctl, btop"
    fi
}

if [ "$INSTALL_DEPS" = true ]; then
    install_dependencies
fi

# Verify dependencies
check_prerequisites() {
    local missing=0
    echo "==> Checking prerequisites..."
    if command -v cargo &>/dev/null; then
        echo "  [OK] cargo: $(cargo --version 2>/dev/null || echo 'installed')"
    else
        echo "  [FAIL] cargo: Rust compiler ('cargo') is required." >&2
        echo "         Install via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
        missing=$((missing + 1))
    fi

    for tool in wpctl playerctl brightnessctl btop; do
        if command -v "$tool" &>/dev/null; then
            echo "  [OK] $tool: installed"
        else
            echo "  [WARN] $tool: not found in PATH (recommended for full island features)." >&2
        fi
    done

    if [ $missing -gt 0 ]; then
        return 1
    fi
    return 0
}

if [ "$CHECK_ONLY" = true ]; then
    check_prerequisites
    echo "==> All required prerequisites satisfied."
    exit 0
fi

if [ "$DRY_RUN" = true ]; then
    echo "==> [DRY-RUN] Simulating Atlantis installation..."
    check_prerequisites
    echo "==> [DRY-RUN] Planned installation targets and actions:"
    echo "  Prefix:              $PREFIX"
    echo "  Binary directory:    $BIN_DIR"
    echo "  Binaries:            $BIN_DIR/atlantis"
    echo "                       $BIN_DIR/atlantis-ctl"
    echo "  Symlinks:            $BIN_DIR/liquid-shell -> atlantis"
    echo "                       $BIN_DIR/island-ctl -> atlantis-ctl"
    echo "  Config directory:    $CONFIG_DIR"
    echo "  Configuration file:  $CONFIG_DIR/config.json"
    if [ "$INSTALL_SYSTEMD" = true ]; then
        echo "  Systemd unit:        $SYSTEMD_USER_DIR/atlantis.service (enable & start)"
    fi
    exit 0
fi

check_prerequisites

# If not local build, clone into temporary directory
if [ "$LOCAL_BUILD" = false ]; then
    BUILD_TEMP="$(mktemp -d)"
    trap 'rm -rf "$BUILD_TEMP"' EXIT
    echo "==> Cloning Atlantis from GitHub..."
    git clone --depth 1 https://github.com/ziuus/atlantis.git "$BUILD_TEMP"
    cd "$BUILD_TEMP"
fi

echo "==> Building Atlantis (release profile)..."
cargo build --release --bin atlantis

echo "==> Installing binaries to $BIN_DIR..."
mkdir -p "$BIN_DIR"
install -Dm755 "target/release/atlantis" "$BIN_DIR/atlantis"
ln -sf atlantis "$BIN_DIR/liquid-shell"

if [ -f "bin/atlantis-ctl" ]; then
    install -Dm755 "bin/atlantis-ctl" "$BIN_DIR/atlantis-ctl"
    ln -sf atlantis-ctl "$BIN_DIR/island-ctl"
elif [ -f "bin/island-ctl" ]; then
    install -Dm755 "bin/island-ctl" "$BIN_DIR/atlantis-ctl"
    ln -sf atlantis-ctl "$BIN_DIR/island-ctl"
fi

echo "==> Ensuring configuration directory at $CONFIG_DIR..."
mkdir -p "$CONFIG_DIR"
if [ ! -f "$CONFIG_DIR/config.json" ]; then
    if [ -f "$HOME/.config/liquid-glass/config.json" ]; then
        cp "$HOME/.config/liquid-glass/config.json" "$CONFIG_DIR/config.json"
        echo "==> Migrated existing configuration from ~/.config/liquid-glass/config.json"
    elif [ -f "config/config.json" ]; then
        cp "config/config.json" "$CONFIG_DIR/config.json"
        echo "==> Created default configuration at $CONFIG_DIR/config.json"
    fi
fi

# Systemd user service installation
if [ "$INSTALL_SYSTEMD" = true ]; then
    echo "==> Installing systemd user service..."
    mkdir -p "$SYSTEMD_USER_DIR"
    cat << EOF > "$SYSTEMD_USER_DIR/atlantis.service"
[Unit]
Description=🔱 Atlantis — Wayland Desktop Environment Shell
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=$BIN_DIR/atlantis
Restart=on-failure
RestartSec=1
Environment=WAYLAND_DISPLAY=wayland-1

[Install]
WantedBy=graphical-session.target
EOF
    ln -sf atlantis.service "$SYSTEMD_USER_DIR/liquid-shell.service"
    systemctl --user daemon-reload
    systemctl --user enable --now atlantis.service
    echo "==> Systemd service installed and active (atlantis.service)"
fi

# Optional XDG autostart entry
if [ "$INSTALL_AUTOSTART" = true ]; then
    mkdir -p "$AUTOSTART_DIR"
    cat << EOF > "$AUTOSTART_DIR/atlantis.desktop"
[Desktop Entry]
Type=Application
Name=Atlantis
Comment=Physical optical lensing desktop environment for Wayland
Exec=$BIN_DIR/atlantis
Terminal=false
Categories=Utility;System;
EOF
    echo "==> Autostart entry installed to $AUTOSTART_DIR/atlantis.desktop"
fi

echo ""
echo "🔱 Atlantis installation complete!"
echo "   Binary: $BIN_DIR/atlantis (symlinked to liquid-shell)"
echo "   CLI:    $BIN_DIR/atlantis-ctl (symlinked to island-ctl)"
echo "   Config: $CONFIG_DIR/config.json"
echo ""
echo "Quick Test:"
echo "   atlantis-ctl toggle"
