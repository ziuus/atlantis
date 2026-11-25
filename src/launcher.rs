//! Atlantis Liquid Glass Launcher — Spotlight / Raycast-grade Wayland app launcher.
//! Scans `.desktop` entries, ranks by launch frequency, evaluates live math expressions,
//! and supports `>` system actions with pure Liquid Glass rendering.

use glass_core::animation::{Spring, SpringParams};
use glass_core::font::{FontWeight, UIFont};
use glass_core::geometry::Shape;
use glass_core::renderer::GlassRenderer;
use glass_core::{GlassSurface, Material, SharedGpu};
use glass_platform::WallpaperManager;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use smithay_client_toolkit::compositor::{CompositorState, SurfaceData};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface, LayerSurfaceConfigure,
    LayerSurfaceData,
};
use smithay_client_toolkit::shell::WaylandSurface;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;
use wayland_backend::client::ObjectId;
use wayland_client::protocol::{wl_region, wl_surface};
use wayland_client::{Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

pub const LAUNCHER_W: u32 = 620;
pub const LAUNCHER_H: u32 = 430;
const MAX_VISIBLE_ROWS: usize = 6;

pub struct WaylandWindow {
    pub surface: wl_surface::WlSurface,
    pub display: *mut std::ffi::c_void,
}

unsafe impl Send for WaylandWindow {}
unsafe impl Sync for WaylandWindow {}

impl HasWindowHandle for WaylandWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let ptr = std::ptr::NonNull::new(self.surface.id().as_ptr() as *mut _).unwrap();
        let handle = WaylandWindowHandle::new(ptr);
        unsafe { Ok(WindowHandle::borrow_raw(RawWindowHandle::Wayland(handle))) }
    }
}

impl HasDisplayHandle for WaylandWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let ptr = std::ptr::NonNull::new(self.display).unwrap();
        let handle = WaylandDisplayHandle::new(ptr);
        unsafe { Ok(DisplayHandle::borrow_raw(RawDisplayHandle::Wayland(handle))) }
    }
}

#[derive(Debug, Clone)]
pub struct DesktopApp {
    pub name: String,
    pub exec: String,
    pub comment: String,
    pub is_action: bool,
}

#[derive(Debug, Clone)]
enum SearchResult {
    Calc { expr: String, value: String },
    App(DesktopApp),
}

pub struct LauncherWidget {
    pub layer_surface: Option<LayerSurface>,
    pub surface: wl_surface::WlSurface,
    pub window_wrapper: Arc<WaylandWindow>,
    pub renderer: Option<GlassRenderer>,

    pub width: u32,
    pub height: u32,
    pub pos_x: i32,
    pub pos_y: i32,

    pub is_open: bool,
    pub open_spring: Spring,
    pub style_mode: f32,
    pub theme_mode: u32,
    pub blur_radius: f32,
    pub anim_speed: f32,
    pub is_wallpaper_light: bool,
    pub wallpaper_crop: Option<Vec<u8>>,

    pub font: UIFont,
    pub overlay_buffer: Vec<u8>,
    pub cursor_pos: [f32; 2],
    pub needs_redraw: bool,
    pub needs_input_region_update: bool,

    pub query: String,
    pub selected_idx: usize,
    pub scroll_offset: usize,
    pub apps: Vec<DesktopApp>,
    pub freq: HashMap<String, u32>,
    filtered: Vec<SearchResult>,
    pub opened_at: Instant,
}

impl LauncherWidget {
    pub fn new<D>(
        qh: &QueueHandle<D>,
        compositor: &CompositorState,
        layer_shell: &LayerShell,
        display_ptr: *mut std::ffi::c_void,
    ) -> Self
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<ZwlrLayerSurfaceV1, LayerSurfaceData>
            + 'static,
    {
        let width = LAUNCHER_W;
        let height = LAUNCHER_H;
        let pos_x = (1920 - width as i32) / 2;
        let pos_y = 220;

        let surface = compositor.create_surface(qh);
        let layer_surface = layer_shell.create_layer_surface(
            qh,
            surface.clone(),
            Layer::Overlay,
            Some("atlantis-launcher"),
            None,
        );
        layer_surface.set_anchor(Anchor::TOP);
        layer_surface.set_size(width, height);
        layer_surface.set_margin(pos_y, 0, 0, 0);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer_surface.commit();

        let window_wrapper = Arc::new(WaylandWindow {
            surface: surface.clone(),
            display: display_ptr,
        });

        let freq = Self::load_freq();
        let apps = Self::scan_desktop_apps();

        let mut w = Self {
            layer_surface: Some(layer_surface),
            surface,
            window_wrapper,
            renderer: None,
            width,
            height,
            pos_x,
            pos_y,
            is_open: false,
            open_spring: Spring::with_params(
                0.0,
                SpringParams {
                    stiffness: 300.0,
                    damping: 34.5,
                    mass: 0.9,
                    initial_velocity: 0.0,
                    epsilon: 0.002,
                },
            ),
            style_mode: 2.0,
            theme_mode: 0,
            blur_radius: 0.0,
            anim_speed: 1.0,
            is_wallpaper_light: false,
            wallpaper_crop: None,
            font: UIFont::new(),
            overlay_buffer: Vec::new(),
            cursor_pos: [-1.0, -1.0],
            needs_redraw: false,
            needs_input_region_update: true,
            query: String::new(),
            selected_idx: 0,
            scroll_offset: 0,
            apps,
            freq,
            filtered: Vec::new(),
            opened_at: Instant::now(),
        };
        w.refilter();
        w
    }

    pub fn surface_id(&self) -> ObjectId {
        self.surface.id()
    }

    pub fn set_screen_geometry(&mut self, screen_w: u32, screen_h: u32) {
        self.pos_x = (screen_w as i32 - self.width as i32) / 2;
        self.pos_y = ((screen_h as i32 - self.height as i32) / 3).max(80);
        if let Some(ls) = &self.layer_surface {
            ls.set_margin(self.pos_y, 0, 0, 0);
            ls.commit();
        }
    }

    pub fn init_gpu(&mut self, gpu: &SharedGpu) {
        let mut renderer = gpu
            .create_renderer(self.window_wrapper.clone(), self.width, self.height)
            .expect("Failed to create GlassRenderer for launcher");
        if let Some(crop) = &self.wallpaper_crop {
            renderer.update_background(self.width, self.height, crop);
        }
        self.renderer = Some(renderer);
    }

    pub fn configure(&mut self, configure: LayerSurfaceConfigure) {
        if configure.new_size.0 != 0 {
            self.width = configure.new_size.0;
        }
        if configure.new_size.1 != 0 {
            self.height = configure.new_size.1;
        }
        if let Some(r) = &mut self.renderer {
            r.resize(self.width, self.height);
            if let Some(crop) = &self.wallpaper_crop {
                r.update_background(self.width, self.height, crop);
            }
        }
    }

    pub fn update_wallpaper(&mut self, crop: Vec<u8>) {
        self.is_wallpaper_light =
            WallpaperManager::is_buffer_light_hysteresis(&crop, self.is_wallpaper_light);
        if let Some(r) = &mut self.renderer {
            r.update_background(self.width, self.height, &crop);
        }
        self.wallpaper_crop = Some(crop);
        if self.is_open {
            self.needs_redraw = true;
        }
    }

    pub fn effective_is_light(&self) -> bool {
        match self.theme_mode {
            1 => false,
            2 => true,
            _ => self.is_wallpaper_light,
        }
    }

    pub fn toggle(&mut self) {
        if self.is_open {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.opened_at = Instant::now();
        self.query.clear();
        self.selected_idx = 0;
        self.scroll_offset = 0;
        if self.apps.is_empty() {
            self.apps = Self::scan_desktop_apps();
        }
        self.refilter();
        if self.open_spring.value < 0.08 {
            self.open_spring.value = 0.08;
        }
        self.open_spring.set_target(1.0);
        if let Some(ls) = &self.layer_surface {
            ls.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        }
        self.needs_input_region_update = true;
        self.needs_redraw = true;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.open_spring.set_target(0.0);
        if let Some(ls) = &self.layer_surface {
            ls.set_keyboard_interactivity(KeyboardInteractivity::None);
        }
        self.needs_input_region_update = true;
        self.needs_redraw = true;
    }

    pub fn apply_input_region<D>(&mut self, compositor: &CompositorState, qh: &QueueHandle<D>)
    where
        D: Dispatch<wl_region::WlRegion, ()> + 'static,
    {
        self.needs_input_region_update = false;
        let region = compositor.wl_compositor().create_region(qh, ());
        if self.is_open || self.open_spring.value > 0.02 {
            region.add(0, 0, self.width as i32, self.height as i32);
        }
        self.surface.set_input_region(Some(&region));
        region.destroy();
        if !self.needs_redraw {
            self.surface.commit();
        }
    }

    pub fn update_physics(&mut self, dt: f32) -> bool {
        let eff_dt = (dt * self.anim_speed).clamp(0.0, 0.05);
        self.open_spring.update(eff_dt);
        if (self.open_spring.value - self.open_spring.target).abs() < 0.002
            && self.open_spring.velocity.abs() < 0.01
        {
            self.open_spring.snap_to_target();
        }
        !self.open_spring.is_at_rest()
    }

    fn freq_path() -> String {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{}/.cache/atlantis/launcher_freq.json", home)
    }

    fn load_freq() -> HashMap<String, u32> {
        std::fs::read_to_string(Self::freq_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save_freq(&self) {
        let path = Self::freq_path();
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(&self.freq) {
            let _ = std::fs::write(path, json);
        }
    }

    fn scan_desktop_apps() -> Vec<DesktopApp> {
        let mut seen = HashMap::new();
        let home = std::env::var("HOME").unwrap_or_default();
        let dirs = [
            format!("{}/.local/share/applications", home),
            "/usr/local/share/applications".to_string(),
            "/usr/share/applications".to_string(),
        ];
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                if let Ok(raw) = std::fs::read_to_string(&path) {
                    if let Some(app) = Self::parse_desktop_file(&raw) {
                        seen.entry(app.name.clone()).or_insert(app);
                    }
                }
            }
        }
        let mut list: Vec<DesktopApp> = seen.into_values().collect();
        list.sort_by_key(|a| a.name.to_lowercase());
        list
    }

    fn parse_desktop_file(raw: &str) -> Option<DesktopApp> {
        let mut in_entry = false;
        let mut name = None;
        let mut exec = None;
        let mut comment = String::new();
        let mut no_display = false;
        let mut app_type = String::new();

        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_entry = trimmed == "[Desktop Entry]";
                continue;
            }
            if !in_entry || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                match k.trim() {
                    "Type" => app_type = v.trim().to_string(),
                    "NoDisplay" | "Hidden" if v.trim().eq_ignore_ascii_case("true") => {
                        no_display = true;
                    }
                    "Name" if name.is_none() => name = Some(v.trim().to_string()),
                    "Exec" if exec.is_none() => {
                        // Strip desktop field codes (%u, %U, %f, %F, etc.)
                        let cleaned: Vec<&str> = v
                            .split_whitespace()
                            .filter(|tok| !tok.starts_with('%'))
                            .collect();
                        if !cleaned.is_empty() {
                            exec = Some(cleaned.join(" "));
                        }
                    }
                    "Comment" if comment.is_empty() => comment = v.trim().to_string(),
                    _ => {}
                }
            }
        }

        if no_display || (!app_type.is_empty() && app_type != "Application") {
            return None;
        }
        Some(DesktopApp {
            name: name?,
            exec: exec?,
            comment,
            is_action: false,
        })
    }

    fn system_actions() -> Vec<DesktopApp> {
        vec![
            DesktopApp {
                name: "Control Center".into(),
                exec: "echo control > /tmp/liquid_island.fifo".into(),
                comment: "Open Atlantis Control Center".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Notifications".into(),
                exec: "echo notif > /tmp/liquid_island.fifo".into(),
                comment: "Show Island Notification History".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Lock Screen".into(),
                exec: "hyprlock".into(),
                comment: "Lock current Wayland session".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Suspend".into(),
                exec: "systemctl suspend".into(),
                comment: "Put system to sleep".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Reboot".into(),
                exec: "systemctl reboot".into(),
                comment: "Restart computer".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Power Off".into(),
                exec: "systemctl poweroff".into(),
                comment: "Shut down computer".into(),
                is_action: true,
            },
        ]
    }

    fn eval_math(expr: &str) -> Option<String> {
        let clean: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
        if clean.is_empty()
            || !clean
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '*' | '/' | '.'))
            || !clean.chars().any(|c| matches!(c, '+' | '-' | '*' | '/'))
        {
            return None;
        }
        // Simple left-to-right precedence-aware evaluator (+,- and *,/)
        let val = eval_expr_add_sub(&clean)?;
        if val.is_finite() {
            if (val - val.round()).abs() < 1e-6 {
                Some(format!("{}", val.round() as i64))
            } else {
                Some(
                    format!("{:.4}", val)
                        .trim_end_matches('0')
                        .trim_end_matches('.')
                        .to_string(),
                )
            }
        } else {
            None
        }
    }

    fn refilter(&mut self) {
        self.filtered.clear();
        let q = self.query.trim().to_lowercase();

        if let Some(ans) = Self::eval_math(&self.query) {
            self.filtered.push(SearchResult::Calc {
                expr: self.query.trim().to_string(),
                value: ans,
            });
        }

        if let Some(cmd_q) = q.strip_prefix('>') {
            let cq = cmd_q.trim();
            for act in Self::system_actions() {
                if cq.is_empty() || act.name.to_lowercase().contains(cq) {
                    self.filtered.push(SearchResult::App(act));
                }
            }
        } else {
            let mut scored: Vec<(i32, &DesktopApp)> = self
                .apps
                .iter()
                .filter_map(|app| {
                    let nl = app.name.to_lowercase();
                    let freq_boost = (*self.freq.get(&app.name).unwrap_or(&0) as i32) * 10;
                    if q.is_empty() {
                        Some((freq_boost, app))
                    } else if nl.starts_with(&q) {
                        Some((1000 + freq_boost, app))
                    } else if nl.contains(&q) {
                        Some((500 + freq_boost, app))
                    } else if app.comment.to_lowercase().contains(&q) {
                        Some((200 + freq_boost, app))
                    } else {
                        None
                    }
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
            for (_, app) in scored.into_iter().take(32) {
                self.filtered.push(SearchResult::App(app.clone()));
            }
        }

        if self.selected_idx >= self.filtered.len() {
            self.selected_idx = 0;
        }
        self.scroll_offset = 0;
    }

    fn activate_selected(&mut self) {
        let Some(item) = self.filtered.get(self.selected_idx).cloned() else {
            return;
        };
        match item {
            SearchResult::Calc { value, .. } => {
                let _ = Command::new("wl-copy").arg(&value).spawn();
                self.close();
            }
            SearchResult::App(app) => {
                *self.freq.entry(app.name.clone()).or_insert(0) += 1;
                self.save_freq();
                let _ = Command::new("sh").args(["-c", &app.exec]).spawn();
                self.close();
            }
        }
    }

    pub fn on_keyboard_event(&mut self, event: &KeyEvent) -> bool {
        if !self.is_open {
            return false;
        }
        match event.keysym {
            Keysym::Escape => {
                self.close();
                true
            }
            Keysym::Return => {
                self.activate_selected();
                true
            }
            Keysym::Down | Keysym::Tab => {
                if !self.filtered.is_empty() {
                    self.selected_idx = (self.selected_idx + 1) % self.filtered.len();
                    if self.selected_idx >= self.scroll_offset + MAX_VISIBLE_ROWS {
                        self.scroll_offset = self.selected_idx - MAX_VISIBLE_ROWS + 1;
                    } else if self.selected_idx == 0 {
                        self.scroll_offset = 0;
                    }
                }
                true
            }
            Keysym::Up => {
                if !self.filtered.is_empty() {
                    self.selected_idx = if self.selected_idx == 0 {
                        self.filtered.len() - 1
                    } else {
                        self.selected_idx - 1
                    };
                    if self.selected_idx < self.scroll_offset {
                        self.scroll_offset = self.selected_idx;
                    } else if self.selected_idx >= self.scroll_offset + MAX_VISIBLE_ROWS {
                        self.scroll_offset = self.selected_idx - MAX_VISIBLE_ROWS + 1;
                    }
                }
                true
            }
            Keysym::BackSpace => {
                self.query.pop();
                self.refilter();
                true
            }
            _ => {
                if let Some(txt) = &event.utf8 {
                    let mut pushed = false;
                    for ch in txt.chars() {
                        if !ch.is_control() {
                            self.query.push(ch);
                            pushed = true;
                        }
                    }
                    if pushed {
                        self.refilter();
                    }
                    pushed
                } else {
                    false
                }
            }
        }
    }

    pub fn on_pointer_event(&mut self, event: &PointerEvent) -> bool {
        if !self.is_open {
            return false;
        }
        let lx = event.position.0 as f32;
        let ly = event.position.1 as f32;
        self.cursor_pos = [lx / (self.width as f32), ly / (self.height as f32)];

        match event.kind {
            PointerEventKind::Motion { .. } => {
                if ly >= 82.0 {
                    let rel = ((ly - 82.0) / 54.0).floor() as usize;
                    let idx = self.scroll_offset + rel;
                    if rel < MAX_VISIBLE_ROWS && idx < self.filtered.len() {
                        self.selected_idx = idx;
                        return true;
                    }
                }
                false
            }
            PointerEventKind::Press { button: 272, .. } => {
                if ly >= 82.0 {
                    let rel = ((ly - 82.0) / 54.0).floor() as usize;
                    let idx = self.scroll_offset + rel;
                    if rel < MAX_VISIBLE_ROWS && idx < self.filtered.len() {
                        self.selected_idx = idx;
                        self.activate_selected();
                        return true;
                    }
                }
                false
            }
            PointerEventKind::Axis { vertical, .. } => {
                let d = if vertical.discrete != 0 {
                    vertical.discrete
                } else if vertical.absolute > 0.5 {
                    1
                } else if vertical.absolute < -0.5 {
                    -1
                } else {
                    0
                };
                if d > 0 && self.selected_idx + 1 < self.filtered.len() {
                    self.selected_idx += 1;
                    if self.selected_idx >= self.scroll_offset + MAX_VISIBLE_ROWS {
                        self.scroll_offset = self.selected_idx - MAX_VISIBLE_ROWS + 1;
                    }
                    return true;
                } else if d < 0 && self.selected_idx > 0 {
                    self.selected_idx -= 1;
                    if self.selected_idx < self.scroll_offset {
                        self.scroll_offset = self.selected_idx;
                    }
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn draw_search_icon(buf: &mut [u8], w: u32, h: u32, cx: f32, cy: f32, col: [u8; 4]) {
        UIFont::draw_rounded_rect_stroke(buf, w, h, cx - 1.5, cy - 1.5, 5.2, 5.2, 5.2, 1.4, col);
        UIFont::draw_segment(
            buf,
            w,
            h,
            (cx + 2.5, cy + 2.5),
            (cx + 6.5, cy + 6.5),
            1.5,
            col,
        );
    }

    fn render_ui_overlay(&mut self) {
        let req_len = (self.width * self.height * 4) as usize;
        if self.overlay_buffer.len() != req_len {
            self.overlay_buffer.resize(req_len, 0);
        }
        self.overlay_buffer.fill(0);
        let t = self.open_spring.value.clamp(0.0, 1.0);
        if t <= 0.02 {
            return;
        }

        let is_light = self.effective_is_light();
        let mut buf_vec = std::mem::take(&mut self.overlay_buffer);
        let buf = &mut buf_vec[..];
        let a = (255.0 * t) as u8;
        let col_ink = if is_light {
            [10, 14, 24, a]
        } else {
            [245, 247, 252, a]
        };
        let col_dim = if is_light {
            [45, 60, 80, (175.0 * t) as u8]
        } else {
            [205, 218, 238, (175.0 * t) as u8]
        };
        let no_shadow = [0, 0, 0, 0];

        // 1. Search Bar
        Self::draw_search_icon(buf, self.width, self.height, 34.0, 40.0, col_dim);
        let display_q = if self.query.is_empty() {
            "Search apps, > commands, or math..."
        } else {
            &self.query
        };
        let q_col = if self.query.is_empty() {
            col_dim
        } else {
            col_ink
        };
        let q_end = self.font.draw_text_centered_y(
            buf,
            self.width,
            self.height,
            display_q,
            56.0,
            40.0,
            16.5,
            q_col,
            no_shadow,
            FontWeight::Medium,
        );
        if !self.query.is_empty() {
            UIFont::draw_rounded_rect_fill(
                buf,
                self.width,
                self.height,
                q_end + 3.0,
                40.0,
                0.8,
                9.0,
                0.5,
                col_ink,
            );
        }

        let count_s = format!("{} results", self.filtered.len());
        let cw = self.font.measure_text(&count_s, 11.0, FontWeight::Medium);
        self.font.draw_text_centered_y(
            buf,
            self.width,
            self.height,
            &count_s,
            self.width as f32 - 26.0 - cw,
            40.0,
            11.0,
            col_dim,
            no_shadow,
            FontWeight::Medium,
        );

        // Subtle divider hairline
        let div_c = if is_light {
            [0, 0, 0, (24.0 * t) as u8]
        } else {
            [255, 255, 255, (28.0 * t) as u8]
        };
        UIFont::draw_rounded_rect_fill(
            buf,
            self.width,
            self.height,
            self.width as f32 * 0.5,
            68.0,
            (self.width as f32 - 44.0) * 0.5,
            0.5,
            0.5,
            div_c,
        );

        // 2. Results list (up to 6 rows)
        let row_w = self.width as f32 - 36.0;
        let row_cx = self.width as f32 * 0.5;
        for slot in 0..MAX_VISIBLE_ROWS {
            let idx = self.scroll_offset + slot;
            let Some(item) = self.filtered.get(idx) else {
                break;
            };
            let ry = 104.0 + (slot as f32) * 54.0;
            let is_sel = idx == self.selected_idx;

            if is_sel {
                let sel_bg = if is_light {
                    [0, 0, 0, (22.0 * t) as u8]
                } else {
                    [255, 255, 255, (32.0 * t) as u8]
                };
                UIFont::draw_rounded_rect_fill(
                    buf,
                    self.width,
                    self.height,
                    row_cx,
                    ry,
                    row_w * 0.5,
                    22.0,
                    12.0,
                    sel_bg,
                );
                let sel_rim = if is_light {
                    [0, 0, 0, (36.0 * t) as u8]
                } else {
                    [255, 255, 255, (50.0 * t) as u8]
                };
                UIFont::draw_rounded_rect_stroke(
                    buf,
                    self.width,
                    self.height,
                    row_cx,
                    ry,
                    row_w * 0.5,
                    22.0,
                    12.0,
                    0.9,
                    sel_rim,
                );
            }

            let (badge, title, sub) = match item {
                SearchResult::Calc { expr, value } => (
                    "=",
                    format!("= {}", value),
                    format!("{} · Enter to copy", expr),
                ),
                SearchResult::App(app) => (
                    if app.is_action { ">" } else { "•" },
                    app.name.clone(),
                    if app.comment.is_empty() {
                        app.exec.clone()
                    } else {
                        app.comment.clone()
                    },
                ),
            };

            let badge_bg = if is_light {
                [0, 0, 0, (16.0 * t) as u8]
            } else {
                [255, 255, 255, (24.0 * t) as u8]
            };
            UIFont::draw_rounded_rect_fill(
                buf,
                self.width,
                self.height,
                40.0,
                ry,
                13.0,
                13.0,
                8.0,
                badge_bg,
            );
            self.font.draw_text_centered(
                buf,
                self.width,
                self.height,
                badge,
                40.0,
                ry,
                12.5,
                col_ink,
                no_shadow,
                FontWeight::Bold,
            );

            let title_trunc: String = title.chars().take(34).collect();
            let sub_trunc: String = sub.chars().take(48).collect();
            self.font.draw_text_centered_y(
                buf,
                self.width,
                self.height,
                &title_trunc,
                64.0,
                ry - 7.0,
                13.5,
                col_ink,
                no_shadow,
                FontWeight::SemiBold,
            );
            self.font.draw_text_centered_y(
                buf,
                self.width,
                self.height,
                &sub_trunc,
                64.0,
                ry + 9.0,
                10.5,
                col_dim,
                no_shadow,
                FontWeight::Medium,
            );

            if is_sel {
                let hint = "Enter";
                let hw = self.font.measure_text(hint, 10.5, FontWeight::Medium);
                self.font.draw_text_centered_y(
                    buf,
                    self.width,
                    self.height,
                    hint,
                    self.width as f32 - 30.0 - hw,
                    ry,
                    10.5,
                    col_dim,
                    no_shadow,
                    FontWeight::Medium,
                );
            }
        }
        self.overlay_buffer = buf_vec;
    }

    pub fn draw(&mut self) -> bool {
        let t = self.open_spring.value.clamp(0.0, 1.0);
        if !self.is_open && t <= 0.01 {
            if let Some(r) = &mut self.renderer {
                self.overlay_buffer.fill(0);
                r.update_overlay(self.width, self.height, &self.overlay_buffer);
                let _ = r.render(&[]);
            }
            return false;
        }

        self.render_ui_overlay();
        let is_light = if self.effective_is_light() { 1.0 } else { 0.0 };

        if let Some(r) = &mut self.renderer {
            r.update_overlay(self.width, self.height, &self.overlay_buffer);
            let motion = (1.0 - t).clamp(0.0, 1.0);
            let inset_x = motion * 0.08;
            let inset_y = motion * 0.06;
            let y_slide = motion * 0.05;
            let w_b = (1.0 - inset_x * 2.0).clamp(0.2, 1.0);
            let h_b = (1.0 - inset_y * 2.0).clamp(0.2, 1.0);
            let x_b = (0.5 - w_b * 0.5).clamp(0.0, 0.8);
            let y_b = (inset_y + y_slide).clamp(0.0, 0.8);

            let card = GlassSurface::new()
                .material(Material::LiquidGlass)
                .shape(Shape::RoundedRectangle { radius: 22.0 })
                .bounds(x_b, y_b, w_b.min(1.0 - x_b), h_b.min(1.0 - y_b))
                .curvature(20.0)
                .bevel(16.0 + motion * 6.0)
                .refraction(20.0 + motion * 16.0)
                .dispersion(0.14)
                .blur(self.blur_radius)
                .specular(1.25)
                .fresnel(0.65)
                .style_mode(self.style_mode)
                .tint(0.0, 0.0, t, is_light)
                .element_type(27.0)
                .pointer_pos(self.cursor_pos[0], self.cursor_pos[1]);
            let _ = r.render(&[card]);
        }
        !self.open_spring.is_at_rest()
    }
}

fn eval_expr_add_sub(s: &str) -> Option<f64> {
    let mut terms = Vec::new();
    let mut ops = Vec::new();
    let mut start = 0usize;
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if (bytes[i] == b'+' || bytes[i] == b'-') && i > start {
            terms.push(eval_expr_mul_div(&s[start..i])?);
            ops.push(bytes[i]);
            start = i + 1;
        }
    }
    terms.push(eval_expr_mul_div(&s[start..])?);
    let mut acc = terms[0];
    for (idx, &op) in ops.iter().enumerate() {
        if op == b'+' {
            acc += terms[idx + 1];
        } else {
            acc -= terms[idx + 1];
        }
    }
    Some(acc)
}

fn eval_expr_mul_div(s: &str) -> Option<f64> {
    let mut factors = Vec::new();
    let mut ops = Vec::new();
    let mut start = 0usize;
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if (bytes[i] == b'*' || bytes[i] == b'/') && i > start {
            factors.push(s[start..i].parse::<f64>().ok()?);
            ops.push(bytes[i]);
            start = i + 1;
        }
    }
    factors.push(s[start..].parse::<f64>().ok()?);
    let mut acc = factors[0];
    for (idx, &op) in ops.iter().enumerate() {
        if op == b'*' {
            acc *= factors[idx + 1];
        } else {
            if factors[idx + 1].abs() < 1e-12 {
                return None;
            }
            acc /= factors[idx + 1];
        }
    }
    Some(acc)
}
