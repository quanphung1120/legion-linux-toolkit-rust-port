# Maintainer: VVAT3R
#
# Legion Linux Toolkit — Rust daemon, Slint GUI/tray and CLI for Lenovo Legion
# laptops. Hardware access uses the upstream lenovo-wmi-* and ideapad_laptop
# kernel drivers; no DKMS module and no LenovoLegionLinux dependency.

pkgname=legion-linux-toolkit
pkgver=1.0.0alpha1
pkgrel=1
pkgdesc="Daemon, GUI, tray and CLI for Lenovo Legion laptops (upstream lenovo-wmi drivers)"
arch=('x86_64')
url="https://github.com/VVAT3R/legion-linux-toolkit"
license=('MIT')
depends=(
    'dbus'
    'polkit'
    'fontconfig'
    'libxkbcommon'
)
makedepends=('cargo' 'pkgconf')
optdepends=(
    'gnome-shell-extension-appindicator: tray icon support on GNOME'
)
# The daemon binary is named legiond, which collides with LenovoLegionLinux's
# C daemon of the same name.
conflicts=('lenovolegionlinux' 'lenovolegionlinux-git')
source=("${pkgname}-${pkgver}.tar.gz::https://github.com/VVAT3R/legion-linux-toolkit/archive/v${pkgver}.tar.gz")
sha256sums=('SKIP')

prepare() {
    cd "${srcdir}/${pkgname}-${pkgver}"
    export RUSTUP_TOOLCHAIN=stable
    cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "${srcdir}/${pkgname}-${pkgver}"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    cargo build --release --locked --workspace
}

check() {
    cd "${srcdir}/${pkgname}-${pkgver}"
    export RUSTUP_TOOLCHAIN=stable
    cargo test --release --locked --workspace
}

package() {
    cd "${srcdir}/${pkgname}-${pkgver}"

    install -Dm755 target/release/legiond    "${pkgdir}/usr/bin/legiond"
    install -Dm755 target/release/legion-ctl "${pkgdir}/usr/bin/legion-ctl"
    install -Dm755 target/release/legion-gui "${pkgdir}/usr/bin/legion-gui"

    install -Dm644 systemd/legiond.service \
        "${pkgdir}/usr/lib/systemd/system/legiond.service"
    install -Dm644 dbus/org.legiontoolkit.Daemon.conf \
        "${pkgdir}/usr/share/dbus-1/system.d/org.legiontoolkit.Daemon.conf"
    install -Dm644 polkit/org.legion-toolkit.policy \
        "${pkgdir}/usr/share/polkit-1/actions/org.legiontoolkit.policy"

    install -Dm644 desktop/legion-toolkit.desktop \
        "${pkgdir}/usr/share/applications/legion-toolkit.desktop"
    install -Dm644 desktop/legion-toolkit-tray.desktop \
        "${pkgdir}/etc/xdg/autostart/legion-toolkit-tray.desktop"

    install -Dm644 logo.png \
        "${pkgdir}/usr/share/icons/hicolor/512x512/apps/legion-toolkit.png"
    # The tray asks its host for icons by name, one per power profile.
    for icon in legion-toolkit legion-toolkit-quiet legion-toolkit-balanced \
                legion-toolkit-performance legion-toolkit-extreme \
                legion-toolkit-custom; do
        install -Dm644 "crates/legion-gui/assets/${icon}.png" \
            "${pkgdir}/usr/share/icons/hicolor/64x64/apps/${icon}.png"
    done

    install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}

# post_install: enable the daemon with
#   sudo systemctl enable --now legiond
