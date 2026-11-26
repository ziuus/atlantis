//! Atlantis Liquid Glass Launcher — Spotlight / Raycast-grade Wayland app launcher.
//! Features a 2-card floating Liquid Glass spatial layout, real `.desktop` PNG app icons
//! with Lucide vector fallbacks, live inspector pane, math evaluator, and `>` system actions.

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
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;
use wayland_backend::client::ObjectId;
use wayland_client::protocol::{wl_region, wl_surface};
use wayland_client::{Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

pub const LAUNCHER_W: u32 = 680;
pub const LAUNCHER_H: u32 = 490;
const SEARCH_CARD_H: f32 = 64.0;
const CARD_GAP: f32 = 12.0;
const MAIN_CARD_Y: f32 = SEARCH_CARD_H + CARD_GAP; // 76.0
const LIST_TOP_Y: f32 = MAIN_CARD_Y + 18.0; // 94.0
const ROW_H: f32 = 56.0;
const DIVIDER_X: f32 = 414.0;
const MAX_VISIBLE_ROWS: usize = 6;
const SMALL_ICON_SZ: u32 = 28;
const LARGE_ICON_SZ: u32 = 52;

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
    pub icon: String,
    pub is_action: bool,
}

#[derive(Debug, Clone)]
enum SearchResult {
    Calc { expr: String, value: String },
    App(DesktopApp),
}

struct CachedIcon {
    small: Vec<u8>, // 28x28 RGBA
    large: Vec<u8>, // 52x52 RGBA
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
    pub is_window_overlapping: bool,
    pub app_backdrop_solid: bool,
    pub power_save: bool,
    pub open_spring: Spring,
    pub panel_spring: Spring,
    pub style_mode: f32,
    pub theme_mode: u32,
    /// Customizable accent color (RGB). Default: Apple-blue [18, 108, 224].
    pub accent_color: [u8; 3],
    /// When true, primary text uses exact #000 / #FFF.
    pub pure_bw: bool,
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
    icon_cache: HashMap<String, Option<CachedIcon>>,
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
        let pos_y = 200;

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
            is_window_overlapping: false,
            app_backdrop_solid: true,
            power_save: false,
            open_spring: Spring::with_params(
                0.0,
                SpringParams {
                    stiffness: 380.0,
                    damping: 33.0,
                    mass: 0.75,
                    initial_velocity: 0.0,
                    epsilon: 0.002,
                },
            ),
            panel_spring: Spring::with_params(
                0.0,
                SpringParams {
                    stiffness: 240.0,
                    damping: 30.0,
                    mass: 1.05,
                    initial_velocity: 0.0,
                    epsilon: 0.002,
                },
            ),
            style_mode: 2.0,
            theme_mode: 0,
            accent_color: [18, 108, 224],
            pure_bw: true,
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
            icon_cache: HashMap::new(),
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

    pub fn effective_style_mode(&self) -> f32 {
        if self.app_backdrop_solid && self.is_window_overlapping {
            0.0
        } else {
            self.style_mode
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
        self.is_window_overlapping = crate::island::IslandWidget::query_hyprland_overlap(
            self.pos_x,
            self.pos_y,
            self.width as i32,
            self.height as i32,
        );
        self.opened_at = Instant::now();
        self.query.clear();
        self.selected_idx = 0;
        self.scroll_offset = 0;
        if self.apps.is_empty() {
            self.apps = Self::scan_desktop_apps();
        }
        self.refilter();
        self.open_spring.set_target(1.0);
        self.panel_spring.set_target(1.0);
        if let Some(ls) = &self.layer_surface {
            ls.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        }
        self.needs_input_region_update = true;
        self.needs_redraw = true;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.open_spring.set_target(0.0);
        self.panel_spring.set_target(0.0);
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
        if self.is_open {
            region.add(0, 0, self.width as i32, self.height as i32);
        }
        self.surface.set_input_region(Some(&region));
        region.destroy();
        self.surface.commit();
    }

    pub fn update_physics(&mut self, dt: f32) -> bool {
        if self.power_save {
            let changed = !self.open_spring.is_at_rest() || !self.panel_spring.is_at_rest();
            self.open_spring.snap_to_target();
            self.panel_spring.snap_to_target();
            if changed {
                self.needs_redraw = true;
                self.needs_input_region_update = true;
            }
            return false;
        }
        let eff_dt = (dt * self.anim_speed).clamp(0.0, 0.05);
        self.open_spring.update(eff_dt);
        self.panel_spring.update(eff_dt);
        if (self.open_spring.value - self.open_spring.target).abs() < 0.002
            && self.open_spring.velocity.abs() < 0.01
        {
            if !self.open_spring.is_at_rest() {
                self.needs_redraw = true;
                self.needs_input_region_update = true;
            }
            self.open_spring.snap_to_target();
        }
        if (self.panel_spring.value - self.panel_spring.target).abs() < 0.002
            && self.panel_spring.velocity.abs() < 0.01
        {
            if !self.panel_spring.is_at_rest() {
                self.needs_redraw = true;
            }
            self.panel_spring.snap_to_target();
        }
        !self.open_spring.is_at_rest() || !self.panel_spring.is_at_rest()
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
        if let Some(parent) = Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(&self.freq) {
            let _ = std::fs::write(path, json);
        }
    }

    fn collect_desktop_files(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
        if depth > 3 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::collect_desktop_files(&path, depth + 1, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("desktop") {
                out.push(path);
            }
        }
    }

    fn scan_desktop_apps() -> Vec<DesktopApp> {
        let mut seen = HashMap::new();
        let home = std::env::var("HOME").unwrap_or_default();
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Ok(xdg_home) = std::env::var("XDG_DATA_HOME") {
            for p in xdg_home.split(':').filter(|s| !s.is_empty()) {
                dirs.push(PathBuf::from(p).join("applications"));
            }
        } else if !home.is_empty() {
            dirs.push(PathBuf::from(&home).join(".local/share/applications"));
        }
        if !home.is_empty() {
            dirs.push(PathBuf::from(&home).join(".local/share/flatpak/exports/share/applications"));
            dirs.push(PathBuf::from(&home).join(".nix-profile/share/applications"));
        }
        if let Ok(xdg_dirs) = std::env::var("XDG_DATA_DIRS") {
            for p in xdg_dirs.split(':').filter(|s| !s.is_empty()) {
                dirs.push(PathBuf::from(p).join("applications"));
            }
        }
        for fallback in [
            "/usr/local/share/applications",
            "/usr/share/applications",
            "/var/lib/flatpak/exports/share/applications",
            "/var/lib/snapd/desktop/applications",
            "/run/current-system/sw/share/applications",
        ] {
            dirs.push(PathBuf::from(fallback));
        }

        let mut visited_dirs = std::collections::HashSet::new();
        let mut files = Vec::new();
        for dir in dirs {
            let canon = std::fs::canonicalize(&dir).unwrap_or(dir);
            if visited_dirs.insert(canon.clone()) {
                Self::collect_desktop_files(&canon, 0, &mut files);
            }
        }
        for path in files {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Some(app) = Self::parse_desktop_file(&raw) {
                    seen.entry(app.name.clone()).or_insert(app);
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
        let mut icon = String::new();
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
                        let cleaned: Vec<&str> = v
                            .split_whitespace()
                            .filter(|tok| !tok.starts_with('%'))
                            .collect();
                        if !cleaned.is_empty() {
                            exec = Some(cleaned.join(" "));
                        }
                    }
                    "Icon" if icon.is_empty() => icon = v.trim().to_string(),
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
            icon,
            is_action: false,
        })
    }

    fn find_icon_png(icon_name: &str) -> Option<PathBuf> {
        if icon_name.is_empty() {
            return None;
        }
        let p = Path::new(icon_name);
        if p.is_absolute() && p.exists() {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "png" || ext == "jpg" || ext == "jpeg" {
                return Some(p.to_path_buf());
            }
            return None;
        }

        let home = std::env::var("HOME").unwrap_or_default();
        let bases = [
            format!("{}/.local/share/icons", home),
            format!("{}/.local/share/flatpak/exports/share/icons", home),
            "/var/lib/flatpak/exports/share/icons".to_string(),
            "/usr/share/icons".to_string(),
        ];
        let themes = ["hicolor", "Papirus", "Papirus-Dark", "Adwaita", "breeze"];
        let sizes = ["64x64", "48x48", "128x128", "256x256", "32x32"];

        for base in &bases {
            for theme in &themes {
                for sz in &sizes {
                    let cand =
                        PathBuf::from(format!("{}/{}/{}/apps/{}.png", base, theme, sz, icon_name));
                    if cand.exists() {
                        return Some(cand);
                    }
                }
            }
        }
        let pixmap = PathBuf::from(format!("/usr/share/pixmaps/{}.png", icon_name));
        if pixmap.exists() {
            return Some(pixmap);
        }

        let pix = PathBuf::from(format!("/usr/share/pixmaps/{}.png", icon_name));
        if pix.exists() {
            return Some(pix);
        }
        let pix_exact = PathBuf::from(format!("/usr/share/pixmaps/{}", icon_name));
        if pix_exact.exists()
            && pix_exact
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("png"))
                .unwrap_or(false)
        {
            return Some(pix_exact);
        }
        None
    }

    fn ensure_icon_cached(&mut self, icon_name: &str) {
        if icon_name.is_empty() || self.icon_cache.contains_key(icon_name) {
            return;
        }
        let loaded = Self::find_icon_png(icon_name).and_then(|path| {
            let img = image::open(path).ok()?;
            let small = img
                .resize_exact(
                    SMALL_ICON_SZ,
                    SMALL_ICON_SZ,
                    image::imageops::FilterType::Triangle,
                )
                .to_rgba8()
                .into_raw();
            let large = img
                .resize_exact(
                    LARGE_ICON_SZ,
                    LARGE_ICON_SZ,
                    image::imageops::FilterType::Triangle,
                )
                .to_rgba8()
                .into_raw();
            Some(CachedIcon { small, large })
        });
        self.icon_cache.insert(icon_name.to_string(), loaded);
    }

    fn blit_rgba_rounded(
        buf: &mut [u8],
        buf_size: (u32, u32),
        rgba: &[u8],
        icon_size: (u32, u32),
        center: (f32, f32),
        radius: f32,
        alpha_mul: f32,
    ) {
        let (bw, bh) = buf_size;
        let (iw, ih) = icon_size;
        let (cx, cy) = center;
        let x0 = (cx - iw as f32 * 0.5).round() as i32;
        let y0 = (cy - ih as f32 * 0.5).round() as i32;
        let hw = iw as f32 * 0.5;
        let hh = ih as f32 * 0.5;

        for sy in 0..ih as i32 {
            let dy = y0 + sy;
            if dy < 0 || dy >= bh as i32 {
                continue;
            }
            for sx in 0..iw as i32 {
                let dx = x0 + sx;
                if dx < 0 || dx >= bw as i32 {
                    continue;
                }
                let lx = (sx as f32 + 0.5 - hw).abs() - (hw - radius);
                let ly = (sy as f32 + 0.5 - hh).abs() - (hh - radius);
                let sdf = lx.max(0.0).hypot(ly.max(0.0)) + lx.max(ly).min(0.0) - radius;
                let mask = (0.5 - sdf).clamp(0.0, 1.0);
                if mask <= 0.001 {
                    continue;
                }

                let si = ((sy as u32 * iw + sx as u32) * 4) as usize;
                let a = (rgba[si + 3] as f32 / 255.0) * mask * alpha_mul;
                if a <= 0.001 {
                    continue;
                }

                let di = ((dy as u32 * bw + dx as u32) * 4) as usize;
                let inv_a = 1.0 - a;
                buf[di] = (rgba[si] as f32 * a + buf[di] as f32 * inv_a).min(255.0) as u8;
                buf[di + 1] =
                    (rgba[si + 1] as f32 * a + buf[di + 1] as f32 * inv_a).min(255.0) as u8;
                buf[di + 2] =
                    (rgba[si + 2] as f32 * a + buf[di + 2] as f32 * inv_a).min(255.0) as u8;
                buf[di + 3] = ((a * 255.0) + buf[di + 3] as f32 * inv_a).min(255.0) as u8;
            }
        }
    }

    fn draw_fallback_icon(
        font: &UIFont,
        buf: &mut [u8],
        buf_size: (u32, u32),
        app: &DesktopApp,
        center: (f32, f32),
        col: [u8; 4],
        large: bool,
    ) {
        let (w, h) = buf_size;
        let (cx, cy) = center;
        let key = format!("{} {}", app.icon, app.name).to_lowercase();
        if key.contains("control center") || key.contains("setting") || key.contains("config") {
            UIFont::draw_settings_icon(buf, w, h, cx, cy, col);
        } else if key.contains("notification") || key.contains("bell") {
            UIFont::draw_bell_icon(buf, w, h, cx, cy, if large { 1.6 } else { 1.0 }, col);
        } else if key.contains("lock") {
            UIFont::draw_lock_icon(buf, w, h, cx, cy, col);
        } else if key.contains("suspend") || key.contains("sleep") || key.contains("moon") {
            UIFont::draw_moon_icon(buf, w, h, cx, cy, col);
        } else if key.contains("reboot") || key.contains("restart") {
            UIFont::draw_restart_icon(buf, w, h, cx, cy, col);
        } else if key.contains("power") || key.contains("shutdown") {
            UIFont::draw_power_icon(buf, w, h, cx, cy, col);
        } else if key.contains("term")
            || key.contains("kitty")
            || key.contains("alacritty")
            || key.contains("wezterm")
            || key.contains("foot")
            || key.contains("konsole")
        {
            UIFont::draw_terminal_icon(buf, w, h, cx, cy, col);
        } else if key.contains("browser")
            || key.contains("firefox")
            || key.contains("chrom")
            || key.contains("brave")
            || key.contains("zen")
        {
            UIFont::draw_browser_icon(buf, w, h, cx, cy, col);
        } else if key.contains("code")
            || key.contains("editor")
            || key.contains("nvim")
            || key.contains("vim")
            || key.contains("antigravity")
            || key.contains("zed")
        {
            UIFont::draw_editor_icon(buf, w, h, cx, cy, col);
        } else if key.contains("file")
            || key.contains("folder")
            || key.contains("thunar")
            || key.contains("dolphin")
            || key.contains("nautilus")
        {
            UIFont::draw_folder_icon(buf, w, h, cx, cy, col);
        } else if key.contains("music")
            || key.contains("spotify")
            || key.contains("vlc")
            || key.contains("audio")
        {
            UIFont::draw_music_icon(buf, w, h, cx, cy, col);
        } else if key.contains("htop") || key.contains("btop") || key.contains("monitor") {
            UIFont::draw_cpu_icon(buf, w, h, cx, cy, col);
        } else if key.contains("calendar") {
            UIFont::draw_calendar_icon(buf, w, h, cx, cy, col);
        } else if key.contains("clock") || key.contains("time") {
            UIFont::draw_clock_icon(buf, w, h, cx, cy, col);
        } else {
            let ch = app
                .name
                .chars()
                .find(|c| c.is_alphanumeric())
                .unwrap_or('•')
                .to_uppercase()
                .to_string();
            font.draw_text_centered(
                buf,
                w,
                h,
                &ch,
                cx,
                cy,
                if large { 22.0 } else { 13.0 },
                col,
                [0, 0, 0, 0],
                FontWeight::Bold,
            );
        }
    }

    fn system_actions() -> Vec<DesktopApp> {
        vec![
            DesktopApp {
                name: "Control Center".into(),
                exec: "echo control > /tmp/liquid_island.fifo".into(),
                comment: "Open Atlantis Control Center".into(),
                icon: "preferences-system".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Notifications".into(),
                exec: "echo notif > /tmp/liquid_island.fifo".into(),
                comment: "Show Island Notification History".into(),
                icon: "notifications".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Lock Screen".into(),
                exec: "command -v hyprlock >/dev/null && hyprlock || command -v swaylock >/dev/null && swaylock || loginctl lock-session".into(),
                comment: "Lock current Wayland session".into(),
                icon: "system-lock-screen".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Suspend".into(),
                exec: "systemctl suspend".into(),
                comment: "Put system to sleep".into(),
                icon: "system-suspend".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Reboot".into(),
                exec: "systemctl reboot".into(),
                comment: "Restart computer".into(),
                icon: "system-reboot".into(),
                is_action: true,
            },
            DesktopApp {
                name: "Power Off".into(),
                exec: "systemctl poweroff".into(),
                comment: "Shut down computer".into(),
                icon: "system-shutdown".into(),
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
                if lx < DIVIDER_X && ly >= LIST_TOP_Y {
                    let rel = ((ly - LIST_TOP_Y) / ROW_H).floor() as usize;
                    let idx = self.scroll_offset + rel;
                    if rel < MAX_VISIBLE_ROWS && idx < self.filtered.len() {
                        self.selected_idx = idx;
                        return true;
                    }
                }
                false
            }
            PointerEventKind::Press { button: 272, .. } => {
                if lx < DIVIDER_X && ly >= LIST_TOP_Y {
                    let rel = ((ly - LIST_TOP_Y) / ROW_H).floor() as usize;
                    let idx = self.scroll_offset + rel;
                    if rel < MAX_VISIBLE_ROWS && idx < self.filtered.len() {
                        self.selected_idx = idx;
                        self.activate_selected();
                        return true;
                    }
                } else if (448.0..=644.0).contains(&lx) && (396.0..=432.0).contains(&ly) {
                    self.activate_selected();
                    return true;
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
        UIFont::draw_rounded_rect_stroke(buf, w, h, cx - 1.5, cy - 1.5, 5.5, 5.5, 5.5, 1.5, col);
        UIFont::draw_segment(
            buf,
            w,
            h,
            (cx + 2.8, cy + 2.8),
            (cx + 7.2, cy + 7.2),
            1.6,
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

        // Pre-cache icons for visible rows + selected item
        let mut icons_to_cache = Vec::new();
        for slot in 0..MAX_VISIBLE_ROWS {
            if let Some(SearchResult::App(app)) = self.filtered.get(self.scroll_offset + slot) {
                icons_to_cache.push(app.icon.clone());
            }
        }
        if let Some(SearchResult::App(app)) = self.filtered.get(self.selected_idx) {
            icons_to_cache.push(app.icon.clone());
        }
        for name in icons_to_cache {
            self.ensure_icon_cached(&name);
        }

        let is_light = self.effective_is_light();
        let mut buf_vec = std::mem::take(&mut self.overlay_buffer);
        let buf = &mut buf_vec[..];
        let a = (255.0 * t) as u8;
        let col_ink = if is_light {
            if self.pure_bw { [0, 0, 0, a] } else { [10, 14, 24, a] }
        } else {
            if self.pure_bw { [255, 255, 255, a] } else { [245, 247, 252, a] }
        };
        let ac = self.accent_color;
        let col_dim = if is_light {
            if self.pure_bw { [0, 0, 0, (175.0 * t) as u8] } else { [42, 56, 76, (175.0 * t) as u8] }
        } else {
            if self.pure_bw { [255, 255, 255, (175.0 * t) as u8] } else { [205, 218, 238, (175.0 * t) as u8] }
        };
        let col_accent = if self.pure_bw {
            if is_light {
                [0, 0, 0, (220.0 * t) as u8]
            } else if ac == [0, 0, 0] {
                [255, 255, 255, (230.0 * t) as u8]
            } else {
                [ac[0], ac[1], ac[2], (230.0 * t) as u8]
            }
        } else if is_light {
            [ac[0], ac[1], ac[2], (220.0 * t) as u8]
        } else {
            // Lighten accent for dark bg: blend toward white
            let lr = (ac[0] as u16 + 97).min(255) as u8;
            let lg = (ac[1] as u16 + 82).min(255) as u8;
            let lb = (ac[2] as u16 + 31).min(255) as u8;
            [lr, lg, lb, (230.0 * t) as u8]
        };
        let pill_bg = if is_light {
            [0, 0, 0, (16.0 * t) as u8]
        } else {
            [255, 255, 255, (24.0 * t) as u8]
        };
        let pill_rim = if is_light {
            [0, 0, 0, (34.0 * t) as u8]
        } else {
            [255, 255, 255, (48.0 * t) as u8]
        };
        let no_shadow = [0, 0, 0, 0];

        // ── CARD 1: TOP FLOATING SEARCH PILL (y: 0..64) ─────────────────────
        let search_cy = SEARCH_CARD_H * 0.5;
        Self::draw_search_icon(buf, self.width, self.height, 30.0, search_cy, col_accent);

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
            52.0,
            search_cy,
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
                search_cy,
                0.9,
                9.5,
                0.5,
                col_accent,
            );
        }

        // Mode badge + count pill on right of search bar
        let mode_label = if self.query.trim().starts_with('>') {
            "Actions"
        } else if Self::eval_math(&self.query).is_some() {
            "Math"
        } else {
            "Apps"
        };
        let badge_s = format!("{} · {}", mode_label, self.filtered.len());
        let bw = self.font.measure_text(&badge_s, 11.0, FontWeight::SemiBold);
        let pill_hw = (bw * 0.5 + 11.0).max(34.0);
        let pill_cx = self.width as f32 - 20.0 - pill_hw;
        UIFont::draw_rounded_rect_fill(
            buf,
            self.width,
            self.height,
            pill_cx,
            search_cy,
            pill_hw,
            12.0,
            10.0,
            pill_bg,
        );
        UIFont::draw_rounded_rect_stroke(
            buf,
            self.width,
            self.height,
            pill_cx,
            search_cy,
            pill_hw,
            12.0,
            10.0,
            0.8,
            pill_rim,
        );
        self.font.draw_text_centered(
            buf,
            self.width,
            self.height,
            &badge_s,
            pill_cx,
            search_cy,
            11.0,
            col_dim,
            no_shadow,
            FontWeight::SemiBold,
        );

        // ── CARD 2: LEFT RESULTS LIST (x: 14..404, y: 94..430) ──────────────
        let row_w = DIVIDER_X - 30.0;
        let row_cx = 14.0 + row_w * 0.5;
        for slot in 0..MAX_VISIBLE_ROWS {
            let idx = self.scroll_offset + slot;
            let Some(item) = self.filtered.get(idx) else {
                break;
            };
            let ry = LIST_TOP_Y + ROW_H * 0.5 + (slot as f32) * ROW_H;
            let is_sel = idx == self.selected_idx;

            if is_sel {
                let sel_bg = if is_light {
                    [0, 0, 0, (22.0 * t) as u8]
                } else {
                    [255, 255, 255, (30.0 * t) as u8]
                };
                UIFont::draw_rounded_rect_fill(
                    buf,
                    self.width,
                    self.height,
                    row_cx,
                    ry,
                    row_w * 0.5,
                    24.0,
                    12.0,
                    sel_bg,
                );
                UIFont::draw_rounded_rect_stroke(
                    buf,
                    self.width,
                    self.height,
                    row_cx,
                    ry,
                    row_w * 0.5,
                    24.0,
                    12.0,
                    0.9,
                    pill_rim,
                );
                // Left accent indicator bar
                UIFont::draw_rounded_rect_fill(
                    buf,
                    self.width,
                    self.height,
                    18.0,
                    ry,
                    1.5,
                    12.0,
                    1.5,
                    col_accent,
                );
            }

            let icon_cx = 40.0;
            UIFont::draw_rounded_rect_fill(
                buf,
                self.width,
                self.height,
                icon_cx,
                ry,
                16.0,
                16.0,
                9.0,
                pill_bg,
            );

            match item {
                SearchResult::Calc { expr, value } => {
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        "=",
                        icon_cx,
                        ry,
                        15.0,
                        col_accent,
                        no_shadow,
                        FontWeight::Bold,
                    );
                    let title_trunc: String = format!("= {}", value).chars().take(24).collect();
                    let sub_trunc: String = format!("{} · Enter to copy", expr)
                        .chars()
                        .take(34)
                        .collect();
                    self.font.draw_text_centered_y(
                        buf,
                        self.width,
                        self.height,
                        &title_trunc,
                        66.0,
                        ry - 7.5,
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
                        66.0,
                        ry + 9.0,
                        10.5,
                        col_dim,
                        no_shadow,
                        FontWeight::Medium,
                    );
                }
                SearchResult::App(app) => {
                    if let Some(Some(cached)) = self.icon_cache.get(&app.icon) {
                        Self::blit_rgba_rounded(
                            buf,
                            (self.width, self.height),
                            &cached.small,
                            (SMALL_ICON_SZ, SMALL_ICON_SZ),
                            (icon_cx, ry),
                            6.5,
                            t,
                        );
                    } else {
                        Self::draw_fallback_icon(
                            &self.font,
                            buf,
                            (self.width, self.height),
                            app,
                            (icon_cx, ry),
                            col_ink,
                            false,
                        );
                    }
                    let title_trunc: String = app.name.chars().take(24).collect();
                    let sub_src = if app.comment.is_empty() {
                        &app.exec
                    } else {
                        &app.comment
                    };
                    let sub_trunc: String = sub_src.chars().take(34).collect();
                    self.font.draw_text_centered_y(
                        buf,
                        self.width,
                        self.height,
                        &title_trunc,
                        66.0,
                        ry - 7.5,
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
                        66.0,
                        ry + 9.0,
                        10.5,
                        col_dim,
                        no_shadow,
                        FontWeight::Medium,
                    );
                }
            }

            if is_sel {
                let hw = self.font.measure_text("↵", 12.0, FontWeight::Bold);
                self.font.draw_text_centered_y(
                    buf,
                    self.width,
                    self.height,
                    "↵",
                    DIVIDER_X - 24.0 - hw,
                    ry,
                    12.0,
                    col_accent,
                    no_shadow,
                    FontWeight::Bold,
                );
            }
        }

        // ── VERTICAL HAIRLINE DIVIDER (x = 414) ─────────────────────────────
        let div_c = if is_light {
            [0, 0, 0, (24.0 * t) as u8]
        } else {
            [255, 255, 255, (28.0 * t) as u8]
        };
        UIFont::draw_rounded_rect_fill(
            buf,
            self.width,
            self.height,
            DIVIDER_X,
            262.0,
            0.5,
            168.0,
            0.5,
            div_c,
        );

        // ── CARD 2: RIGHT INSPECTOR PANE (x: 414..680, cx = 547) ────────────
        let insp_cx = (DIVIDER_X + self.width as f32) * 0.5;
        if let Some(sel) = self.filtered.get(self.selected_idx) {
            // Large squircle showcase badge
            let badge_cy = 158.0;
            UIFont::draw_rounded_rect_fill(
                buf,
                self.width,
                self.height,
                insp_cx,
                badge_cy,
                34.0,
                34.0,
                16.0,
                pill_bg,
            );
            UIFont::draw_rounded_rect_stroke(
                buf,
                self.width,
                self.height,
                insp_cx,
                badge_cy,
                34.0,
                34.0,
                16.0,
                1.0,
                pill_rim,
            );

            match sel {
                SearchResult::Calc { expr, value } => {
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        "=",
                        insp_cx,
                        badge_cy,
                        28.0,
                        col_accent,
                        no_shadow,
                        FontWeight::Bold,
                    );
                    let val_s: String = value.chars().take(16).collect();
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        &val_s,
                        insp_cx,
                        214.0,
                        18.0,
                        col_ink,
                        no_shadow,
                        FontWeight::Bold,
                    );
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        "Calculator",
                        insp_cx,
                        242.0,
                        11.0,
                        col_accent,
                        no_shadow,
                        FontWeight::SemiBold,
                    );
                    let expr_s: String = expr.chars().take(28).collect();
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        &expr_s,
                        insp_cx,
                        284.0,
                        12.0,
                        col_dim,
                        no_shadow,
                        FontWeight::Medium,
                    );
                }
                SearchResult::App(app) => {
                    if let Some(Some(cached)) = self.icon_cache.get(&app.icon) {
                        Self::blit_rgba_rounded(
                            buf,
                            (self.width, self.height),
                            &cached.large,
                            (LARGE_ICON_SZ, LARGE_ICON_SZ),
                            (insp_cx, badge_cy),
                            11.0,
                            t,
                        );
                    } else {
                        Self::draw_fallback_icon(
                            &self.font,
                            buf,
                            (self.width, self.height),
                            app,
                            (insp_cx, badge_cy),
                            col_ink,
                            true,
                        );
                    }
                    let title_s: String = app.name.chars().take(20).collect();
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        &title_s,
                        insp_cx,
                        214.0,
                        15.5,
                        col_ink,
                        no_shadow,
                        FontWeight::Bold,
                    );

                    let kind_s = if app.is_action {
                        "System Action"
                    } else {
                        "Application"
                    };
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        kind_s,
                        insp_cx,
                        240.0,
                        11.0,
                        col_accent,
                        no_shadow,
                        FontWeight::SemiBold,
                    );

                    let desc_src = if app.comment.is_empty() {
                        "Launch desktop application"
                    } else {
                        &app.comment
                    };
                    let line1: String = desc_src.chars().take(28).collect();
                    let line2: String = desc_src.chars().skip(28).take(28).collect();
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        &line1,
                        insp_cx,
                        276.0,
                        11.5,
                        col_dim,
                        no_shadow,
                        FontWeight::Medium,
                    );
                    if !line2.is_empty() {
                        self.font.draw_text_centered(
                            buf,
                            self.width,
                            self.height,
                            &line2,
                            insp_cx,
                            294.0,
                            11.5,
                            col_dim,
                            no_shadow,
                            FontWeight::Medium,
                        );
                    }

                    let exec_s: String = app.exec.chars().take(26).collect();
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        &exec_s,
                        insp_cx,
                        326.0,
                        10.0,
                        col_dim,
                        no_shadow,
                        FontWeight::Regular,
                    );

                    let count = *self.freq.get(&app.name).unwrap_or(&0);
                    let freq_s = if count == 1 {
                        "Launched 1 time".to_string()
                    } else {
                        format!("Launched {} times", count)
                    };
                    self.font.draw_text_centered(
                        buf,
                        self.width,
                        self.height,
                        &freq_s,
                        insp_cx,
                        364.0,
                        10.5,
                        col_dim,
                        no_shadow,
                        FontWeight::Medium,
                    );
                }
            }

            // Primary Launch / Copy pill button at bottom of Inspector
            let btn_cy = 414.0;
            UIFont::draw_rounded_rect_fill(
                buf,
                self.width,
                self.height,
                insp_cx,
                btn_cy,
                96.0,
                16.0,
                11.0,
                pill_bg,
            );
            UIFont::draw_rounded_rect_stroke(
                buf,
                self.width,
                self.height,
                insp_cx,
                btn_cy,
                96.0,
                16.0,
                11.0,
                0.9,
                pill_rim,
            );
            let btn_label = match sel {
                SearchResult::Calc { .. } => "Copy Result · Enter",
                SearchResult::App(a) if a.is_action => "Run Action · Enter",
                SearchResult::App(_) => "Launch App · Enter",
            };
            self.font.draw_text_centered(
                buf,
                self.width,
                self.height,
                btn_label,
                insp_cx,
                btn_cy,
                11.5,
                col_ink,
                no_shadow,
                FontWeight::SemiBold,
            );
        }

        // ── BOTTOM FOOTER BAR (y: 446..490) ─────────────────────────────────
        UIFont::draw_rounded_rect_fill(
            buf,
            self.width,
            self.height,
            self.width as f32 * 0.5,
            446.0,
            (self.width as f32 - 36.0) * 0.5,
            0.5,
            0.5,
            div_c,
        );
        self.font.draw_text_centered(
            buf,
            self.width,
            self.height,
            "↑↓ Navigate   ·   Enter Launch   ·   > System Actions   ·   2+2 Math   ·   Esc Close",
            self.width as f32 * 0.5,
            468.0,
            10.5,
            col_dim,
            no_shadow,
            FontWeight::Medium,
        );

        self.overlay_buffer = buf_vec;
    }

    pub fn draw(&mut self) -> bool {
        let t_search = self.open_spring.value.clamp(0.0, 1.0);
        let t_panel = ((self.panel_spring.value - 0.12) / 0.88).clamp(0.0, 1.0);
        if !self.is_open && t_search <= 0.01 && t_panel <= 0.01 {
            if let Some(r) = &mut self.renderer {
                self.overlay_buffer.fill(0);
                r.update_overlay(self.width, self.height, &self.overlay_buffer);
                let _ = r.render(&[]);
            }
            return false;
        }

        self.render_ui_overlay();
        let is_light = if self.effective_is_light() { 1.0 } else { 0.0 };

        let eff_style = self.effective_style_mode();
        if let Some(r) = &mut self.renderer {
            r.update_overlay(self.width, self.height, &self.overlay_buffer);
            let h_total = self.height as f32;

            // Card 1 (Search Pill): expands horizontally from center & drops in crisply
            let m_search = (1.0 - t_search).clamp(0.0, 1.0);
            let w_search = (0.42 + 0.58 * t_search).clamp(0.25, 1.0);
            let x_search = (0.5 - w_search * 0.5).clamp(0.0, 0.75);
            let y_search = (m_search * 0.035).clamp(0.0, 0.8);
            let h_search = ((SEARCH_CARD_H / h_total) * (0.76 + 0.24 * t_search)).clamp(0.04, 1.0);

            let search_card = GlassSurface::new()
                .material(Material::LiquidGlass)
                .shape(Shape::RoundedRectangle { radius: 20.0 })
                .bounds(x_search, y_search, w_search.min(1.0 - x_search), h_search)
                .curvature(18.0)
                .bevel(14.0 + m_search * 6.0)
                .refraction(18.0 + m_search * 16.0)
                .dispersion(0.14)
                .blur(self.blur_radius)
                .specular(1.25)
                .fresnel(0.65)
                .style_mode(eff_style)
                .tint(0.0, 0.0, t_search, is_light)
                .element_type(27.0)
                .pointer_pos(self.cursor_pos[0], self.cursor_pos[1]);

            if t_panel > 0.015 {
                // Card 2 (Split Panel): unfolds downward from beneath Search Pill
                let m_panel = (1.0 - t_panel).clamp(0.0, 1.0);
                let w_panel = (0.84 + 0.16 * t_panel).clamp(0.3, 1.0);
                let x_panel = (0.5 - w_panel * 0.5).clamp(0.0, 0.7);
                let main_y_norm = ((MAIN_CARD_Y / h_total) - m_panel * 0.045).clamp(0.0, 0.85);
                let full_main_h = (h_total - MAIN_CARD_Y) / h_total;
                let main_h_norm = (full_main_h * (0.18 + 0.82 * t_panel)).min(1.0 - main_y_norm);

                let main_card = GlassSurface::new()
                    .material(Material::LiquidGlass)
                    .shape(Shape::RoundedRectangle { radius: 24.0 })
                    .bounds(
                        x_panel,
                        main_y_norm,
                        w_panel.min(1.0 - x_panel),
                        main_h_norm,
                    )
                    .curvature(20.0)
                    .bevel(16.0 + m_panel * 8.0)
                    .refraction(20.0 + m_panel * 18.0)
                    .dispersion(0.14)
                    .blur(self.blur_radius)
                    .specular(1.25)
                    .fresnel(0.65)
                    .style_mode(eff_style)
                    .tint(0.0, 0.0, t_panel, is_light)
                    .element_type(27.0)
                    .pointer_pos(self.cursor_pos[0], self.cursor_pos[1]);

                let _ = r.render(&[search_card, main_card]);
            } else {
                let _ = r.render(&[search_card]);
            }
        }
        !self.open_spring.is_at_rest() || !self.panel_spring.is_at_rest()
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
