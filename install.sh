#!/usr/bin/env bash
# Legion Linux Toolkit — Installer (Rust)
# First-class: Ubuntu 26.04 + GNOME. Also supports Arch Linux.
set -euo pipefail

GREEN='\033[0;32m'; CYAN='\033[0;36m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
ok()   { echo -e "  ${GREEN}✓${NC}  $*"; }
info() { echo -e "  ${CYAN}→${NC}  $*"; }
warn() { echo -e "  ${YELLOW}⚠${NC}  $*"; }
err()  { echo -e "  ${RED}✗${NC}  $*" >&2; exit 1; }

[[ $EUID -ne 0 ]] && err "Run as root: sudo bash install.sh"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REAL_USER="${SUDO_USER:-}"

# ── 1. Build dependencies ────────────────────────────────────────────────────
# Slint's Linux prerequisites, per slint-ui/slint docs/building.md: fontconfig
# is needed by every renderer, xkbcommon/xcb by the X11 backend we enable
# alongside Wayland. Without them the cargo build fails at yeslogic-fontconfig-sys.
info "Checking build dependencies…"
if command -v apt-get &>/dev/null; then
    PKGS=()
    command -v cargo      &>/dev/null || PKGS+=(cargo)
    command -v cc         &>/dev/null || PKGS+=(build-essential)
    command -v pkg-config &>/dev/null || PKGS+=(pkg-config)
    pkg-config --exists fontconfig 2>/dev/null || PKGS+=(libfontconfig-dev)
    pkg-config --exists xkbcommon  2>/dev/null || PKGS+=(libxkbcommon-dev)
    pkg-config --exists xcb-shape  2>/dev/null || PKGS+=(libxcb-shape0-dev)
    pkg-config --exists xcb-xfixes 2>/dev/null || PKGS+=(libxcb-xfixes0-dev)
    if [[ ${#PKGS[@]} -gt 0 ]]; then
        apt-get update -qq
        apt-get install -y --no-install-recommends "${PKGS[@]}"
    fi
elif command -v pacman &>/dev/null; then
    pacman -S --noconfirm --needed rust base-devel fontconfig libxkbcommon libxcb
fi
command -v cargo &>/dev/null || err "cargo not found — install Rust via your package manager or https://rustup.rs"
ok "Build dependencies ready"

# ── 2. Sanity-check the kernel interface ─────────────────────────────────────
# The kernel numbers platform-profile-N by registration order, so match on the
# driver name rather than an index (LenovoLegionLinux registers one too).
if ! grep -qx lenovo-wmi-gamezone /sys/class/platform-profile/*/name 2>/dev/null; then
    warn "No lenovo-wmi-gamezone platform-profile device found. This toolkit needs"
    warn "the upstream lenovo-wmi-gamezone driver (kernel 6.17+; 6.19+ recommended)."
    warn "Installing anyway — features will be hidden until the driver is present."
fi

# ── 3. Build (as the invoking user, so ~/.cargo stays user-owned) ────────────
info "Building release binaries…"
if [[ -n "$REAL_USER" ]]; then
    sudo -u "$REAL_USER" cargo build --release --workspace
else
    cargo build --release --workspace
fi
ok "Build complete"

# ── 4. Remove artifacts of the old Python/LLL toolkit (pre-1.0) ──────────────
legacy_cleanup() {
    pkill -f legion-tray.py 2>/dev/null || true
    pkill -f legion-gui.py  2>/dev/null || true
    # Old Python app tree + CLI shims
    rm -rf /usr/lib/legion-toolkit
    rm -f  /usr/local/bin/legion-ctl /usr/local/bin/legion_cli /usr/bin/legion_cli
    # LLL force-load config and DKMS module (never worked on the 82WM)
    rm -f  /etc/modprobe.d/legion_laptop_force.conf
    if command -v dkms &>/dev/null && dkms status 2>/dev/null | grep -q "legion-laptop"; then
        dkms remove -m legion-laptop -v 1.0 --all 2>/dev/null || true
        rm -rf /usr/src/legion-laptop-1.0
        info "Removed legion-laptop DKMS module (superseded by upstream drivers)"
    fi
    # World-writable udev rules and old polkit wiring
    rm -f /etc/udev/rules.d/99-legion-toolkit.rules
    rm -f /etc/polkit-1/rules.d/49-legion-toolkit.rules
    rm -f /usr/share/polkit-1/rules.d/49-legion-toolkit.rules
    udevadm control --reload-rules 2>/dev/null || true
    # (The old installer also chmod'ed sysfs files world-writable; sysfs
    # permissions reset on every boot, so there is nothing to undo there.)
}
info "Cleaning up any previous Python/LLL installation…"
legacy_cleanup
ok "Legacy cleanup done"

# ── 5. Install ───────────────────────────────────────────────────────────────
info "Installing…"
make -C "$SCRIPT_DIR" install
ok "Files installed"

# ── 6. Enable the daemon ─────────────────────────────────────────────────────
systemctl daemon-reload
systemctl enable --now legiond.service
ok "legiond running ($(systemctl is-active legiond.service))"

# ── 7. Start the tray for the invoking user ──────────────────────────────────
if [[ -n "$REAL_USER" ]]; then
    sudo -u "$REAL_USER" nohup /usr/bin/legion-gui --tray >/dev/null 2>&1 &
    ok "Tray started (autostarts on next login)"
fi

echo -e "\n${GREEN}✓ Installation complete.${NC}"
echo -e "  Daemon : systemctl status legiond"
echo -e "  CLI    : legion-ctl status"
echo -e "  GUI    : legion-gui  (tray: legion-gui --tray)"
