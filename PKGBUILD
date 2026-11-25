# Maintainer: ziuus <https://github.com/ziuus>
pkgname=atlantis
pkgver=0.1.0
pkgrel=1
pkgdesc="Turnkey physical optical glass desktop environment and layer-shell rice for Wayland"
arch=('x86_64' 'aarch64')
url="https://github.com/ziuus/atlantis"
license=('MIT')
depends=(
    'wayland'
    'wayland-protocols'
    'libxkbcommon'
    'vulkan-icd-loader'
    'wireplumber'
    'playerctl'
)
optdepends=(
    'brightnessctl: display brightness controls in dynamic island'
    'btop: system resource monitoring launcher'
    'hyprlock: lock screen trigger in power menu'
    'foot: default lightweight terminal emulator'
)
makedepends=('cargo' 'git')
source=("git+https://github.com/ziuus/atlantis.git")
sha256sums=('SKIP')

prepare() {
    cd "$srcdir/$pkgname"
    cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "$srcdir/$pkgname"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    cargo build --frozen --release --bin atlantis
}

check() {
    cd "$srcdir/$pkgname"
    cargo test --frozen
}

package() {
    cd "$srcdir/$pkgname"
    install -Dm755 "target/release/atlantis" "$pkgdir/usr/bin/atlantis"
    ln -sf atlantis "$pkgdir/usr/bin/liquid-shell"

    install -Dm755 "bin/atlantis-ctl" "$pkgdir/usr/bin/atlantis-ctl"
    ln -sf atlantis-ctl "$pkgdir/usr/bin/island-ctl"

    install -Dm644 "config/config.json" "$pkgdir/etc/atlantis/config.json"
    install -Dm644 "systemd/atlantis.service" "$pkgdir/usr/lib/systemd/user/atlantis.service"
}
