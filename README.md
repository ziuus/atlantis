# Liquid Shell (Wayland Desktop Rice)

Turnkey physical optical lensing desktop environment and layer-shell rice for Wayland compositors (Hyprland, Sway, River, Wayfire, Labwc, Niri), powered by the [Liquid Glass Physical Rendering Engine](https://github.com/ziuus/liquid-glass).

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

## 🚀 Installation

### Automated Turnkey Installer

```bash
git clone https://github.com/ziuus/liquid-shell.git
cd liquid-shell
./install.sh --systemd
```

### Options

```bash
./install.sh --check      # Verify dependencies without installing
./install.sh --systemd    # Install binary and enable systemd user service
./install.sh --autostart  # Install XDG desktop autostart entry
./install.sh --uninstall  # Clean removal
```

---

## 🎮 CLI Control (`island-ctl`)

Control the island from your keyboard shortcuts, Hyprland binds, or terminal:

```bash
island-ctl --toggle           # Toggle expand / collapse
island-ctl --menu music       # Jump directly to Music player
island-ctl --menu system      # Jump to System Overview
island-ctl --menu apps        # Jump to Quick Access app drawer
island-ctl --menu power       # Jump to Power menu
island-ctl --cycle-theme      # Cycle Auto -> Dark -> Light
island-ctl --cycle-style      # Cycle Obsidian -> Frosted -> Liquid Glass
```

---

## ⚙️ Core Engine

Liquid Shell is built on the **Liquid Glass Engine**:
- **Repository**: [https://github.com/ziuus/liquid-glass](https://github.com/ziuus/liquid-glass)
- Pure Rust, headless WGPU + WGSL physical optical rendering pipeline with RK4 spring physics.
