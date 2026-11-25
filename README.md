<div align="center">

# 🔱 Atlantis

### Turnkey Physical Optical Glass Desktop Environment for Wayland

[![Wayland](https://img.shields.io/badge/Wayland-Layer--Shell-2B3A42?style=flat-square&logo=linux)](https://wayland.freedesktop.org)
[![Engine](https://img.shields.io/badge/Engine-Liquid--Glass-blue?style=flat-square)](https://github.com/ziuus/liquid-glass)
[![License](https://img.shields.io/badge/License-MIT-black?style=flat-square)](LICENSE)

<br />

<img src="assets/hero.png" alt="Atlantis Liquid Glass Desktop Environment" width="100%" />

*Complete physical optical glass desktop suite with Snell refraction, Cauchy chromatic dispersion, tactile spring kinetics, and dynamic ambient lighting.*

<br />

</div>

---

## 🔮 Components

1. **Liquid Dynamic Island**:
   - Floating compact capsule expanding on hover or click with fluid RK4 spring physics.
   - Interactive media player with spinning vinyl disc, progress scrub bar, and animated equalizer.
   - Quick PipeWire audio sink switcher (`wpctl`) with live device toast notification.
   - Wheel scroll volume control directly over the compact pill.
   - Dynamic status dots: live network connectivity dot (emerald online, amber offline) and battery level.
   - Hardware metrics & system overview (CPU, RAM, Uptime, Battery) with one-click `btop` launcher.
   - Quick Access application drawer with app launchers.
   - Safe Power menu with hold-to-confirm reboot and shutdown actions.

2. **Center Hero Clock**:
   - Giant 3D liquid glass numerals rendered via pure physical Snell refraction.
   - Crystalline optical contrast adapting dynamically to dark, light, and vibrant wallpapers.
   - 360-degree Fresnel perimeter rim lighting with harmonic wallpaper light-guiding.

3. **Liquid Glass App Dock**:
   - Bottom-docked glassmorphism bar with pinned application launchers and launch-glow indicators.

4. **Interactive Todo & Agenda Island**:
   - Top-right floating glass island with live task management and calendar integration.

---

## 🚀 Turnkey Installation

### One-Line Automated Installer

```bash
curl -sSL https://raw.githubusercontent.com/ziuus/atlantis/master/install.sh | bash
```

### From Source

```bash
git clone https://github.com/ziuus/atlantis.git
cd atlantis
./install.sh --systemd
```

### Arch Linux (PKGBUILD)

```bash
cd atlantis
makepkg -si
```

---

## 🎮 CLI Control (`atlantis-ctl`)

Control the desktop from your keyboard shortcuts, Hyprland binds, or terminal:

```bash
atlantis-ctl toggle           # Toggle island visibility
atlantis-ctl music            # Jump directly to Music player
atlantis-ctl system           # Jump to System Overview
atlantis-ctl apps             # Jump to Quick Access app drawer
atlantis-ctl power            # Jump to Power menu
atlantis-ctl theme cycle      # Cycle Auto -> Dark -> Light
atlantis-ctl style cycle      # Cycle Obsidian -> Frosted -> Liquid Glass
```

---

## ⚙️ Core Engine

Atlantis is built on the **Liquid Glass Physical Rendering Engine**:
- **Repository**: [https://github.com/ziuus/liquid-glass](https://github.com/ziuus/liquid-glass)
- Pure Rust, headless WGPU + WGSL physical optical rendering pipeline with RK4 spring physics.
