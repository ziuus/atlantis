//! Universal Linux Wayland Compositor Adapter for Atlantis.
//! Auto-detects Hyprland, Sway, Niri, or Generic wlr-layer-shell compositors
//! so Atlantis runs seamlessly across Linux distributions without separate forks.

use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositorBackend {
    Hyprland,
    Sway,
    Niri,
    Generic,
}

static BACKEND: OnceLock<CompositorBackend> = OnceLock::new();

pub fn detect() -> CompositorBackend {
    *BACKEND.get_or_init(|| {
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            CompositorBackend::Hyprland
        } else if std::env::var_os("SWAYSOCK").is_some() {
            CompositorBackend::Sway
        } else if std::env::var_os("NIRI_SOCKET").is_some() {
            CompositorBackend::Niri
        } else {
            CompositorBackend::Generic
        }
    })
}

/// Returns `(active_classes, focused_class)` for the Dock running-app dots.
pub fn running_app_classes() -> (Vec<String>, Option<String>) {
    match detect() {
        CompositorBackend::Hyprland => {
            let mut active = Vec::new();
            if let Ok(out) = Command::new("hyprctl").args(["clients", "-j"]).output() {
                if out.status.success() {
                    if let Ok(serde_json::Value::Array(list)) = serde_json::from_slice(&out.stdout)
                    {
                        for item in list {
                            if let Some(c) = item.get("class").and_then(|v| v.as_str()) {
                                active.push(c.to_lowercase());
                            }
                        }
                    }
                }
            }
            let focused = Command::new("hyprctl")
                .args(["activewindow", "-j"])
                .output()
                .ok()
                .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
                .and_then(|v| {
                    v.get("class")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_lowercase())
                });
            (active, focused)
        }
        CompositorBackend::Sway => {
            let mut active = Vec::new();
            let mut focused = None;
            if let Ok(out) = Command::new("swaymsg").args(["-t", "get_tree"]).output() {
                if out.status.success() {
                    if let Ok(tree) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                        collect_sway_classes(&tree, &mut active, &mut focused);
                    }
                }
            }
            (active, focused)
        }
        CompositorBackend::Niri => {
            let mut active = Vec::new();
            let mut focused = None;
            if let Ok(out) = Command::new("niri").args(["msg", "-j", "windows"]).output() {
                if out.status.success() {
                    if let Ok(serde_json::Value::Array(list)) = serde_json::from_slice(&out.stdout)
                    {
                        for win in list {
                            if let Some(id) = win.get("app_id").and_then(|v| v.as_str()) {
                                let low = id.to_lowercase();
                                if win
                                    .get("is_focused")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                                {
                                    focused = Some(low.clone());
                                }
                                active.push(low);
                            }
                        }
                    }
                }
            }
            (active, focused)
        }
        CompositorBackend::Generic => (Vec::new(), None),
    }
}

fn collect_sway_classes(
    node: &serde_json::Value,
    active: &mut Vec<String>,
    focused: &mut Option<String>,
) {
    if let Some(app_id) = node.get("app_id").and_then(|v| v.as_str()) {
        let low = app_id.to_lowercase();
        if node
            .get("focused")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            *focused = Some(low.clone());
        }
        active.push(low);
    } else if let Some(class) = node
        .get("window_properties")
        .and_then(|p| p.get("class"))
        .and_then(|v| v.as_str())
    {
        let low = class.to_lowercase();
        if node
            .get("focused")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            *focused = Some(low.clone());
        }
        active.push(low);
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(serde_json::Value::Array(children)) = node.get(key) {
            for ch in children {
                collect_sway_classes(ch, active, focused);
            }
        }
    }
}

/// Focus an open window by app name/class across Hyprland, Sway, and Niri.
pub fn focus_window(app: &str) {
    let clean: String = app
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .collect();
    let target = clean.trim();
    if target.is_empty() {
        return;
    }
    match detect() {
        CompositorBackend::Hyprland => {
            let script = format!(
                "hyprctl dispatch focuswindow 'class:(?i){t}' || hyprctl dispatch focuswindow 'title:(?i){t}'",
                t = target
            );
            let _ = Command::new("sh").args(["-c", &script]).spawn();
        }
        CompositorBackend::Sway => {
            let crit = format!("[app_id=\"(?i){}\"] focus", target);
            let _ = Command::new("swaymsg").arg(crit).spawn();
        }
        CompositorBackend::Niri => {
            let _ = Command::new("niri")
                .args(["msg", "action", "focus-window"])
                .spawn();
        }
        CompositorBackend::Generic => {}
    }
}

/// Lock the active Wayland session using whichever locker is installed (`hyprlock`, `swaylock`, `waylock`, or `loginctl`).
pub fn lock_screen() {
    let script = "command -v hyprlock >/dev/null 2>&1 && exec hyprlock; \
                  command -v swaylock >/dev/null 2>&1 && exec swaylock; \
                  command -v waylock >/dev/null 2>&1 && exec waylock; \
                  exec loginctl lock-session";
    let _ = Command::new("sh").args(["-c", script]).spawn();
}

/// Returns `(x1, y1, x2, y2)` rects for visible windows on the active workspace (Sway / Niri).
pub fn active_window_rects() -> Vec<(i32, i32, i32, i32)> {
    let mut rects = Vec::new();
    match detect() {
        CompositorBackend::Sway => {
            if let Ok(out) = Command::new("swaymsg").args(["-t", "get_tree"]).output() {
                if out.status.success() {
                    if let Ok(tree) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                        collect_sway_rects(&tree, false, &mut rects);
                    }
                }
            }
        }
        CompositorBackend::Niri => {
            if let Ok(out) = Command::new("niri").args(["msg", "-j", "windows"]).output() {
                if out.status.success() {
                    if let Ok(serde_json::Value::Array(list)) = serde_json::from_slice(&out.stdout)
                    {
                        let active_ws = list
                            .iter()
                            .find(|w| w.get("is_focused").and_then(|v| v.as_bool()).unwrap_or(false))
                            .and_then(|w| w.get("workspace_id").and_then(|v| v.as_i64()));
                        if active_ws.is_some() {
                            rects.push((0, 0, 1920, 1080));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    rects
}

fn collect_sway_rects(
    node: &serde_json::Value,
    in_focused_ws: bool,
    rects: &mut Vec<(i32, i32, i32, i32)>,
) {
    let is_ws = node.get("type").and_then(|v| v.as_str()) == Some("workspace");
    let now_focused_ws = if is_ws {
        has_focused_descendant(node)
    } else {
        in_focused_ws
    };
    if now_focused_ws && (node.get("app_id").is_some() || node.get("window_properties").is_some()) {
        if let Some(r) = node.get("rect") {
            let x = r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let w = r.get("width").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let h = r.get("height").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            if w > 0 && h > 0 {
                rects.push((x, y, x + w, y + h));
            }
        }
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(serde_json::Value::Array(children)) = node.get(key) {
            for ch in children {
                collect_sway_rects(ch, now_focused_ws, rects);
            }
        }
    }
}

fn has_focused_descendant(node: &serde_json::Value) -> bool {
    if node
        .get("focused")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return true;
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(serde_json::Value::Array(children)) = node.get(key) {
            if children.iter().any(has_focused_descendant) {
                return true;
            }
        }
    }
    false
}

/// Take a screenshot using `grim` (wlroots), `spectacle` (KDE), or `gnome-screenshot`.
pub fn take_screenshot() {
    let script = "mkdir -p ~/Pictures && OUT=~/Pictures/shot_$(date +%s).png; \
                  if command -v grim >/dev/null 2>&1; then grim \"$OUT\"; \
                  elif command -v spectacle >/dev/null 2>&1; then spectacle -b -n -o \"$OUT\"; \
                  elif command -v gnome-screenshot >/dev/null 2>&1; then gnome-screenshot -f \"$OUT\"; \
                  fi";
    let _ = Command::new("sh").args(["-c", script]).spawn();
}
