#!/usr/bin/env bash
# Legion Linux Toolkit — Uninstaller (Rust)
set -euo pipefail

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
ok()   { echo -e "  ${GREEN}✓${NC}  $*"; }
info() { echo -e "  ${CYAN}→${NC}  $*"; }
err()  { echo -e "  ${RED}✗${NC}  $*" >&2; exit 1; }

[[ $EUID -ne 0 ]] && err "Run as root: sudo bash uninstall.sh"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

info "Stopping processes and daemon…"
pkill -x legion-gui 2>/dev/null || true
systemctl disable --now legiond.service 2>/dev/null || true

info "Removing installed files…"
make -C "$SCRIPT_DIR" uninstall
rm -rf /var/lib/legiond
systemctl daemon-reload

# Also remove anything a pre-1.0 Python/LLL install left behind
info "Removing legacy Python/LLL artifacts (if any)…"
pkill -f legion-tray.py 2>/dev/null || true
pkill -f legion-gui.py  2>/dev/null || true
rm -rf /usr/lib/legion-toolkit
rm -f  /usr/local/bin/legion-ctl /usr/local/bin/legion_cli /usr/bin/legion_cli
rm -f  /etc/modprobe.d/legion_laptop_force.conf
rm -f  /etc/udev/rules.d/99-legion-toolkit.rules
rm -f  /etc/polkit-1/rules.d/49-legion-toolkit.rules
rm -f  /usr/share/polkit-1/rules.d/49-legion-toolkit.rules
if command -v dkms &>/dev/null && dkms status 2>/dev/null | grep -q "legion-laptop"; then
    dkms remove -m legion-laptop -v 1.0 --all 2>/dev/null || true
    rm -rf /usr/src/legion-laptop-1.0
fi
udevadm control --reload-rules 2>/dev/null || true

ok "Legion Linux Toolkit uninstalled."
