/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::collections::HashMap;
#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos"
))]
use std::fs;
#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos"
))]
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use accesskit::Affine;
use dpi::PhysicalSize;
use egui::emath::GuiRounding;
use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    Color32, CornerRadius, FontDefinitions, FontId, Id, Key, Label, LayerId, Margin, Modifiers,
    Order, PaintCallback, Panel, Stroke, StrokeKind, UiBuilder, Vec2, WidgetInfo, WidgetType, pos2,
};
#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos"
))]
use egui::{FontData, FontFamily};
use egui_glow::{CallbackFn, EguiGlow};
use egui_winit::EventResponse;
use euclid::{Length, Point2D, Rect, Scale, Size2D};
#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos"
))]
use log::info;
use log::warn;
use servo::{
    DeviceIndependentPixel, DevicePixel, Image, LoadStatus, OffscreenRenderingContext, PixelFormat,
    RenderingContext, WebView, WebViewId,
};
use url::Url;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::Window;

use crate::desktop::event_loop::AppEvent;
use crate::desktop::headed_window;
use crate::running_app_state::{RunningAppState, UserInterfaceCommand};
use crate::window::ServoShellWindow;

/// The user interface of a headed servoshell. Currently this is implemented via
/// egui.
pub struct Gui {
    rendering_context: Rc<OffscreenRenderingContext>,
    context: EguiGlow,
    toolbar_height: Length<f32, DeviceIndependentPixel>,

    location: String,

    /// Whether the location has been edited by the user without clicking Go.
    location_dirty: bool,

    /// The [`LoadStatus`] of the active `WebView`.
    load_status: LoadStatus,

    /// The text to display in the status bar on the bottom of the window.
    status_text: Option<String>,

    /// Whether or not the current `WebView` can navigate backward.
    can_go_back: bool,

    /// Whether or not the current `WebView` can navigate forward.
    can_go_forward: bool,

    /// Handle to the GPU texture of the favicon.
    ///
    /// These need to be cached across egui draw calls.
    favicon_textures: HashMap<WebViewId, (egui::TextureHandle, egui::load::SizedTexture)>,

    /// AccessKit tree updates pending the next egui tick.
    /// This allows us to ensure that graft nodes are sent before the subtrees they graft.
    pending_accesskit_updates: Vec<accesskit::TreeUpdate>,
}

const TAB_STRIP_COLOR: Color32 = Color32::from_rgb(28, 29, 33);
const TOOLBAR_COLOR: Color32 = Color32::from_rgb(37, 38, 43);
const CHROME_FOREGROUND: Color32 = Color32::from_rgb(242, 243, 245);
const TAB_STRIP_HEIGHT: f32 = 42.0;
const CHROME_ICON_FONT: &str = "servo-chrome-icons";

#[derive(Clone, Copy)]
enum CaptionIcon {
    Minimize,
    Maximize,
    Restore,
    Close,
}

#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos"
))]
fn load_cjk_fonts(font_candidates: &[(&str, &str)]) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut loaded_font_names = Vec::new();

    for (path_str, font_name) in font_candidates.iter() {
        let font_path = Path::new(path_str);
        if font_path.exists() {
            match fs::read(font_path) {
                Ok(bytes) => {
                    if !fonts.font_data.contains_key(*font_name) {
                        fonts
                            .font_data
                            .insert(font_name.to_string(), Arc::new(FontData::from_owned(bytes)));
                        loaded_font_names.push(font_name.to_string());
                        info!("Loaded font: {}", font_name);
                    }
                },
                Err(error) => {
                    info!("Failed to read font {}: {}", font_name, error);
                },
            }
        }
    }

    if !loaded_font_names.is_empty() {
        let proportional = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
        for font_name in loaded_font_names.iter() {
            proportional.insert(0, font_name.clone());
        }
    }

    fonts
}

#[cfg(target_os = "windows")]
fn configure_fonts() -> FontDefinitions {
    // Avoid loading massive CJK font files (~34MB: msyh.ttc 20MB + malgun.ttf 14MB) into RAM for the basic egui chrome.
    // The web content itself uses DirectWrite/dwrote for complete font rendering.
    let mut fonts = load_cjk_fonts(&[
        (r"C:\Windows\Fonts\segoeui.ttf", "Segoe UI"),
    ]);
    let windows_dir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    // Use the installed Windows 11 outlines, without bundling a proprietary font.
    // Windows 10 can fall back to the compatible MDL2 glyph mapping.
    for filename in ["SegoeIcons.ttf", "segmdl2.ttf"] {
        let path = Path::new(&windows_dir).join("Fonts").join(filename);
        if let Ok(bytes) = fs::read(path) {
            fonts.font_data.insert(CHROME_ICON_FONT.into(), Arc::new(FontData::from_owned(bytes)));
            fonts.families.insert(
                FontFamily::Name(CHROME_ICON_FONT.into()),
                vec![CHROME_ICON_FONT.into()],
            );
            break;
        }
    }
    fonts
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn configure_fonts() -> FontDefinitions {
    load_cjk_fonts(&[
        (
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "Noto Sans CJK",
        ), // Ubuntu/Debian
        (
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "Noto Sans CJK",
        ), // Fedora/Arch
        // FreeBSD splits the Noto CJK fonts into regional subsets
        (
            "/usr/local/share/fonts/noto/NotoSansCJKhk-Regular.otf",
            "Noto Sans CJK HK",
        ),
        (
            "/usr/local/share/fonts/noto/NotoSansCJKjp-Regular.otf",
            "Noto Sans CJK JP",
        ),
        (
            "/usr/local/share/fonts/noto/NotoSansCJKkr-Regular.otf",
            "Noto Sans CJK KR",
        ),
        (
            "/usr/local/share/fonts/noto/NotoSansCJKsc-Regular.otf",
            "Noto Sans CJK SC",
        ),
        (
            "/usr/local/share/fonts/noto/NotoSansCJKtc-Regular.otf",
            "Noto Sans CJK TC",
        ),
        (
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "WenQuanYi Micro Hei",
        ), // common fallback
        (
            "/usr/local/share/fonts/wqy/wqy-microhei.ttc",
            "WenQuanYi Micro Hei",
        ), // FreeBSD
    ])
}

#[cfg(target_os = "macos")]
fn configure_fonts() -> FontDefinitions {
    load_cjk_fonts(&[
        (
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
            "Apple SD Gothic Neo",
        ), // Korean
        ("/System/Library/Fonts/STHeiti Medium.ttc", "STHeiti Medium"), // Chinese + Japanese
    ])
}

impl Drop for Gui {
    fn drop(&mut self) {
        self.rendering_context
            .make_current()
            .expect("Could not make window RenderingContext current");
        self.context.destroy();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavIcon {
    Back,
    Forward,
    Reload,
    Stop,
}

impl Gui {
    pub(crate) fn new(
        winit_window: &Window,
        event_loop: &ActiveEventLoop,
        event_loop_proxy: EventLoopProxy<AppEvent>,
        rendering_context: Rc<OffscreenRenderingContext>,
        initial_url: Url,
    ) -> Self {
        rendering_context
            .make_current()
            .expect("Could not make window RenderingContext current");
        let mut context = EguiGlow::new(
            event_loop,
            rendering_context.glow_gl_api(),
            None,
            None,
            false,
        );

        let font_definitions = configure_fonts();
        context.egui_ctx.set_fonts(font_definitions);
        // The browser chrome is dark even when Windows uses a light app theme.
        context.egui_ctx.set_theme(egui::Theme::Dark);

        context
            .egui_winit
            .init_accesskit(event_loop, winit_window, event_loop_proxy);
        winit_window.set_visible(true);

        context.egui_ctx.options_mut(|options| {
            // Disable the builtin egui handlers for the Ctrl+Plus, Ctrl+Minus and Ctrl+0
            // shortcuts as they don't work well with servoshell's `device-pixel-ratio` CLI argument.
            options.zoom_with_keyboard = false;

            options.fallback_theme = egui::Theme::Dark;
        });

        Self {
            rendering_context,
            context,
            toolbar_height: Default::default(),
            location: initial_url.to_string(),
            location_dirty: false,
            load_status: LoadStatus::Complete,
            status_text: None,
            can_go_back: false,
            can_go_forward: false,
            favicon_textures: Default::default(),
            pending_accesskit_updates: vec![],
        }
    }

    pub(crate) fn has_keyboard_focus(&self) -> bool {
        self.context
            .egui_ctx
            .memory(|memory| memory.focused().is_some())
    }

    pub(crate) fn surrender_focus(&self) {
        self.context.egui_ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
    }

    pub(crate) fn on_window_event(
        &mut self,
        winit_window: &Window,
        event: &WindowEvent,
    ) -> EventResponse {
        self.context.on_window_event(winit_window, event)
    }

    /// The height of the top toolbar of this user inteface ie the distance from the top of the
    /// window to the position of the `WebView`.
    pub(crate) fn toolbar_height(&self) -> Length<f32, DeviceIndependentPixel> {
        self.toolbar_height
    }

    /// Return true iff the given position is over the egui toolbar.
    pub(crate) fn is_in_egui_toolbar_rect(
        &self,
        position: Point2D<f32, DeviceIndependentPixel>,
    ) -> bool {
        position.y < self.toolbar_height.get()
    }

    /// Paint the system icon outlines at the current DPI, centered by their ink bounds.
    /// Glyph mapping: https://learn.microsoft.com/windows/apps/design/iconography/segoe-fluent-icons-font
    fn fluent_icon(ui: &egui::Ui, center: egui::Pos2, glyph: &str, size: f32, color: Color32) -> bool {
        let font = FontId::new(size, egui::FontFamily::Name(CHROME_ICON_FONT.into()));
        if !ui.fonts_mut(|fonts| fonts.families().contains(&font.family) && fonts.has_glyphs(&font, glyph)) {
            return false;
        }
        let galley = ui.painter().layout_no_wrap(glyph.to_owned(), font, color);
        let origin = (center - galley.mesh_bounds.center().to_vec2()).round_to_pixels(ui.pixels_per_point());
        ui.painter().galley(origin, galley, color);
        true
    }

    /// Navigation icons share a 16px system glyph grid and circular hover effect.
    fn nav_button(
        ui: &mut egui::Ui,
        icon: NavIcon,
        enabled: bool,
        tooltip: &str,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(30.0, 30.0), egui::Sense::click());
        let hovered = enabled && response.hovered();
        let active = enabled && response.is_pointer_button_down_on();
        let bg = if active {
            Color32::from_rgba_unmultiplied(255, 255, 255, 38)
        } else if hovered {
            Color32::from_rgba_unmultiplied(255, 255, 255, 20)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 15.0, bg);

        let fg = if !enabled {
            Color32::from_rgba_unmultiplied(190, 195, 205, 75)
        } else if hovered {
            Color32::WHITE
        } else {
            Color32::from_gray(215)
        };
        let center = rect.center();

        let glyph = match icon {
            NavIcon::Back => "\u{e72b}",
            NavIcon::Forward => "\u{e72a}",
            NavIcon::Reload => "\u{e72c}",
            NavIcon::Stop => "\u{e711}",
        };
        if Self::fluent_icon(ui, center, glyph, 16.0, fg) {
            return if enabled { response.on_hover_text(tooltip) } else { response };
        }

        match icon {
            NavIcon::Back => {
                let stroke = Stroke::new(1.6_f32, fg);
                ui.painter().line_segment(
                    [center + Vec2::new(4.5, 0.0), center + Vec2::new(-5.0, 0.0)],
                    stroke,
                );
                ui.painter().line_segment(
                    [center + Vec2::new(-1.0, -4.5), center + Vec2::new(-5.0, 0.0)],
                    stroke,
                );
                ui.painter().line_segment(
                    [center + Vec2::new(-1.0, 4.5), center + Vec2::new(-5.0, 0.0)],
                    stroke,
                );
            },
            NavIcon::Forward => {
                let stroke = Stroke::new(1.6_f32, fg);
                ui.painter().line_segment(
                    [center + Vec2::new(-4.5, 0.0), center + Vec2::new(5.0, 0.0)],
                    stroke,
                );
                ui.painter().line_segment(
                    [center + Vec2::new(1.0, -4.5), center + Vec2::new(5.0, 0.0)],
                    stroke,
                );
                ui.painter().line_segment(
                    [center + Vec2::new(1.0, 4.5), center + Vec2::new(5.0, 0.0)],
                    stroke,
                );
            },
            NavIcon::Reload => {
                let stroke = Stroke::new(1.5_f32, fg);
                let radius = 5.2;
                let start_angle = 0.4_f32;
                let end_angle = 4.8_f32;
                let steps = 32;
                let mut prev = egui::pos2(
                    center.x + radius * start_angle.cos(),
                    center.y + radius * start_angle.sin(),
                );
                for i in 1..=steps {
                    let angle = start_angle + (end_angle - start_angle) * (i as f32 / steps as f32);
                    let next = egui::pos2(center.x + radius * angle.cos(), center.y + radius * angle.sin());
                    ui.painter().line_segment([prev, next], stroke);
                    prev = next;
                }
                let tip = prev;
                // Clockwise arrowhead pointing right and downward
                ui.painter().line_segment(
                    [tip, tip + Vec2::new(-3.5, -2.5)],
                    stroke,
                );
                ui.painter().line_segment(
                    [tip, tip + Vec2::new(-3.5, 2.5)],
                    stroke,
                );
            },
            NavIcon::Stop => {
                let stroke = Stroke::new(1.5_f32, fg);
                let arm = 4.0;
                ui.painter().line_segment(
                    [center + Vec2::new(-arm, -arm), center + Vec2::new(arm, arm)],
                    stroke,
                );
                ui.painter().line_segment(
                    [center + Vec2::new(-arm, arm), center + Vec2::new(arm, -arm)],
                    stroke,
                );
            },
        }

        if enabled {
            response.on_hover_text(tooltip)
        } else {
            response
        }
    }

    /// Create a modern tab strip '+' button with hover highlights.
    fn tab_strip_button(ui: &mut egui::Ui, tooltip: &str) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), egui::Sense::click());
        let hovered = response.hovered();
        let active = response.is_pointer_button_down_on();
        let bg = if active {
            Color32::from_rgba_unmultiplied(255, 255, 255, 35)
        } else if hovered {
            Color32::from_rgba_unmultiplied(255, 255, 255, 18)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 14.0, bg);
        let fg = if hovered {
            Color32::WHITE
        } else {
            Color32::from_gray(190)
        };
        let center = rect.center();
        if Self::fluent_icon(ui, center, "\u{e710}", 12.0, fg) {
            return response.on_hover_text(tooltip);
        }
        let stroke = Stroke::new(1.3_f32, fg);
        let arm = 4.5;
        ui.painter().line_segment(
            [center + Vec2::new(-arm, 0.0), center + Vec2::new(arm, 0.0)],
            stroke,
        );
        ui.painter().line_segment(
            [center + Vec2::new(0.0, -arm), center + Vec2::new(0.0, arm)],
            stroke,
        );

        response.on_hover_text(tooltip)
    }

    /// Caption controls use an explicit dark palette, independent of the OS theme.
    fn caption_button(ui: &mut egui::Ui, icon: CaptionIcon, label: &str) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(46.0, TAB_STRIP_HEIGHT),
            egui::Sense::click(),
        );
        let pressed = response.is_pointer_button_down_on();
        let background = if response.hovered() || pressed {
            match (icon, pressed) {
                // Keep the custom chrome consistent with the dark tab strip. The
                // Windows-red hover fill was visually overwhelming at this scale.
                (CaptionIcon::Close, true) => Color32::from_rgb(92, 48, 54),
                (CaptionIcon::Close, false) => Color32::from_rgb(70, 43, 48),
                (_, true) => Color32::from_white_alpha(36),
                (_, false) => Color32::from_white_alpha(22),
            }
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, background);
        let glyph = match icon {
            CaptionIcon::Minimize => "\u{e921}",
            CaptionIcon::Maximize => "\u{e922}",
            CaptionIcon::Restore => "\u{e923}",
            CaptionIcon::Close => "\u{e8bb}",
        };
        if Self::fluent_icon(ui, rect.center(), glyph, 10.0, CHROME_FOREGROUND) {
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
            return response.on_hover_text(label);
        }
        // Rectangles use inside strokes, aligned to physical pixels at any DPI.
        let origin = (rect.center() - Vec2::splat(5.0)).round_to_pixels(ui.pixels_per_point());
        let pixel = 1.0 / ui.pixels_per_point();
        let weight = (ui.pixels_per_point().round() * pixel).max(pixel);
        let stroke = Stroke::new(weight, CHROME_FOREGROUND);
        match icon {
            CaptionIcon::Minimize => {
                painter.rect_filled(
                    egui::Rect::from_min_size(origin + Vec2::new(0.0, 5.0), Vec2::new(10.0, weight)),
                    0.0,
                    CHROME_FOREGROUND,
                );
            },
            CaptionIcon::Maximize => {
                painter.rect_stroke(
                    egui::Rect::from_min_size(origin, Vec2::splat(10.0)),
                    0.0, stroke, StrokeKind::Inside,
                );
            },
            CaptionIcon::Restore => {
                // Two 8px squares with the hidden edges of the rear square omitted.
                painter.line_segment([origin + Vec2::new(2.0, 0.5), origin + Vec2::new(9.5, 0.5)], stroke);
                painter.line_segment([origin + Vec2::new(9.5, 0.5), origin + Vec2::new(9.5, 8.0)], stroke);
                painter.rect_stroke(
                    egui::Rect::from_min_size(origin + Vec2::new(0.0, 2.0), Vec2::splat(8.0)),
                    0.0, stroke, StrokeKind::Inside,
                );
            },
            CaptionIcon::Close => {
                let start = origin + Vec2::splat(weight * 0.5);
                let end = origin + Vec2::splat(10.0 - weight * 0.5);
                painter.line_segment([start, end], stroke);
                painter.line_segment([pos2(start.x, end.y), pos2(end.x, start.y)], stroke);
            },
        }
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
        response.on_hover_text(label)
    }

    /// Draw a tab with a width-aware title and a stable close-button gutter.
    fn browser_tab(
        ui: &mut egui::Ui,
        window: &ServoShellWindow,
        webview: WebView,
        favicon_texture: Option<egui::load::SizedTexture>,
        width: f32,
        reveal_active: bool,
    ) {
        let label = match (webview.page_title(), webview.url()) {
            (Some(title), _) if !title.is_empty() => title,
            (_, Some(url)) => url.to_string(),
            _ => "New Tab".into(),
        };

        let active = window.active_webview().map(|webview| webview.id()) == Some(webview.id());

        let tab_size = Vec2::new(width, TAB_STRIP_HEIGHT);
        let (slot_rect, response) = ui.allocate_exact_size(tab_size, egui::Sense::click());
        let tab_rect = slot_rect.shrink2(Vec2::new(0.0, 5.0));
        if active && reveal_active {
            ui.scroll_to_rect(slot_rect, Some(egui::Align::Center));
        }

        let is_hovered = response.hovered();
        let active_bg = Color32::from_rgb(62, 63, 70);
        let inactive_hover_bg = Color32::from_rgb(43, 44, 50);

        let corner_radius = CornerRadius::same(8);
        if active {
            ui.painter().rect_filled(tab_rect, corner_radius, active_bg);
        } else if is_hovered {
            ui.painter().rect_filled(tab_rect, corner_radius, inactive_hover_bg);
        } else {
            // Subtle 1px vertical separator line between inactive tabs on the right edge
            let sep_x = tab_rect.right();
            let sep_top = tab_rect.top() + 9.0;
            let sep_bottom = tab_rect.bottom() - 9.0;
            ui.painter().line_segment(
                [egui::pos2(sep_x, sep_top), egui::pos2(sep_x, sep_bottom)],
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 28)),
            );
        }

        // 1. Favicon (or crisp vector wireframe globe icon if None)
        let favicon_rect = egui::Rect::from_center_size(
            egui::pos2(tab_rect.left() + 18.0, tab_rect.center().y),
            Vec2::new(16.0, 16.0),
        );
        if let Some(favicon) = favicon_texture {
            ui.painter().image(
                favicon.id,
                favicon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else if !Self::fluent_icon(ui, favicon_rect.center(), "\u{e774}", 14.0, Color32::from_gray(170)) {
            let center = favicon_rect.center();
            let stroke = Stroke::new(1.1_f32, Color32::from_gray(155));
            let r = 5.5_f32;
            ui.painter().circle_stroke(center, r, stroke);
            // Equator (horizontal line)
            ui.painter().line_segment(
                [center + Vec2::new(-r, 0.0), center + Vec2::new(r, 0.0)],
                stroke,
            );
            // Prime meridian (vertical line)
            ui.painter().line_segment(
                [center + Vec2::new(0.0, -r), center + Vec2::new(0.0, r)],
                stroke,
            );
            // Curved side meridians
            let steps = 6;
            let mut prev_left = center + Vec2::new(0.0, -r);
            let mut prev_right = center + Vec2::new(0.0, -r);
            for i in 1..=steps {
                let t = -1.0 + 2.0 * (i as f32 / steps as f32);
                let y = center.y + r * t;
                let dx = 2.4 * (1.0 - t * t).max(0.0).sqrt();
                let next_left = pos2(center.x - dx, y);
                let next_right = pos2(center.x + dx, y);
                ui.painter().line_segment([prev_left, next_left], stroke);
                ui.painter().line_segment([prev_right, next_right], stroke);
                prev_left = next_left;
                prev_right = next_right;
            }
        }

        // 2. Circular Close Button (only shown when active or hovered to keep inactive tabs uncluttered)
        let show_close = width >= 140.0 || active || is_hovered;
        let mut close_clicked = false;
        if show_close {
            let close_rect = egui::Rect::from_center_size(
                egui::pos2(tab_rect.right() - 16.0, tab_rect.center().y),
                Vec2::new(18.0, 18.0),
            );
            let close_resp = ui.interact(
                close_rect,
                ui.id().with(("close_tab", webview.id())),
                egui::Sense::click(),
            );
            let close_hover = close_resp.hovered();
            let close_active = close_resp.is_pointer_button_down_on();

            if close_active {
                ui.painter().rect_filled(close_rect, 9.0, Color32::from_rgba_unmultiplied(255, 255, 255, 50));
            } else if close_hover {
                ui.painter().rect_filled(close_rect, 9.0, Color32::from_rgba_unmultiplied(255, 255, 255, 25));
            }

            let cross_fg = if close_hover {
                Color32::WHITE
            } else if active {
                Color32::from_gray(210)
            } else {
                Color32::from_gray(160)
            };
            let c_center = close_rect.center();
            let arm = 3.5;
            let c_stroke = Stroke::new(1.2_f32, cross_fg);
            ui.painter().line_segment(
                [c_center + Vec2::new(-arm, -arm), c_center + Vec2::new(arm, arm)],
                c_stroke,
            );
            ui.painter().line_segment(
                [c_center + Vec2::new(-arm, arm), c_center + Vec2::new(arm, -arm)],
                c_stroke,
            );
            if close_resp.clicked() || close_resp.middle_clicked() {
                close_clicked = true;
            }
        }

        // 3. Tab Title Text
        let text_color = if active {
            Color32::from_rgb(240, 240, 245)
        } else if is_hovered {
            Color32::from_gray(225)
        } else {
            Color32::from_rgb(175, 175, 185)
        };
        let mut title_job = egui::text::LayoutJob::simple_singleline(
            label.clone(), FontId::proportional(12.0), text_color,
        );
        title_job.wrap.max_width = (tab_rect.width() - 68.0).max(0.0);
        title_job.wrap.max_rows = 1;
        title_job.wrap.break_anywhere = true;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(title_job));
        let title_pos = pos2(tab_rect.left() + 34.0, tab_rect.center().y - galley.size().y * 0.5);
        ui.painter().galley(title_pos, galley, text_color);

        // Tooltip on tab hover
        response.clone().on_hover_ui(|ui| {
            ui.label(&label);
        });

        if close_clicked || response.middle_clicked() {
            window.queue_user_interface_command(UserInterfaceCommand::CloseWebView(webview.id()));
        } else if !active && response.clicked() {
            window.activate_webview(webview.id());
        }
    }

    /// Update the user interface, but do not paint the updated state.
    pub(crate) fn update(
        &mut self,
        state: &RunningAppState,
        window: &ServoShellWindow,
        headed_window: &headed_window::HeadedWindow,
    ) {
        self.rendering_context
            .make_current()
            .expect("Could not make RenderingContext current");
        let Self {
            rendering_context,
            context,
            toolbar_height,
            location,
            location_dirty,
            favicon_textures,
            ..
        } = self;

        let winit_window = headed_window.winit_window();
        context.run(winit_window, |ctx| {
            load_pending_favicons(ctx, window, favicon_textures);

            let show_window_controls = !winit_window.is_decorated();

            // Show toolbar unless fullscreen is from document (web API)
            if !headed_window.is_fullscreen_from_document() {
                // 1. A modern Tab header strip / Title bar at the very top of the window (like Brave/Chrome)
                let tabs_frame = egui::Frame::default()
                    .fill(TAB_STRIP_COLOR)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    });

                Panel::top("tabs")
                    .exact_size(TAB_STRIP_HEIGHT)
                    .show_separator_line(false)
                    .frame(tabs_frame)
                    .show_inside(ctx, |ui| {
                    ui.allocate_ui_with_layout(
                        ui.available_size(),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing = Vec2::ZERO;
                            if show_window_controls {
                                if Self::caption_button(ui, CaptionIcon::Close, "Close window").clicked() {
                                    window.schedule_close();
                                }
                                let is_max = winit_window.is_maximized();
                                let (icon, label) = if is_max {
                                    (CaptionIcon::Restore, "Restore window")
                                } else {
                                    (CaptionIcon::Maximize, "Maximize window")
                                };
                                if Self::caption_button(ui, icon, label).clicked() {
                                    winit_window.set_maximized(!is_max);
                                }
                                if Self::caption_button(ui, CaptionIcon::Minimize, "Minimize window").clicked() {
                                    winit_window.set_minimized(true);
                                }
                            }

                            // Remaining space (tabs, +, and draggable titlebar area)
                            ui.allocate_ui_with_layout(
                                ui.available_size(),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);

                                    let webviews = window.webviews();
                                    let count = webviews.len().max(1) as f32;
                                    // Reserve the new-tab button and a small draggable area even
                                    // when tabs overflow. Caption controls never scroll away.
                                    let available = (ui.available_width() - 76.0).max(0.0);
                                    let tab_width = ((available - (count - 1.0) * 4.0) / count).clamp(88.0, 240.0);
                                    let tabs_width = (count * (tab_width + 4.0) - 4.0).min(available);
                                    let active_id = window.active_webview().map(|view| view.id());
                                    let selection_key = ui.id().with("visible_tab");
                                    let reveal_active = ui.data_mut(|data| {
                                        let changed = data.get_temp::<Option<WebViewId>>(selection_key) != Some(active_id);
                                        data.insert_temp(selection_key, active_id);
                                        changed
                                    });
                                    egui::ScrollArea::horizontal()
                                        .id_salt("browser_tabs")
                                        .max_width(tabs_width)
                                        .max_height(TAB_STRIP_HEIGHT)
                                        .auto_shrink([true, false])
                                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                                        .show(ui, |ui| {
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(count * (tab_width + 4.0) - 4.0, TAB_STRIP_HEIGHT),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                for (id, webview) in webviews {
                                                    let favicon = favicon_textures.get(&id).map(|(_, favicon)| *favicon);
                                                    ui.push_id(id, |ui| {
                                                        Self::browser_tab(ui, window, webview, favicon, tab_width, reveal_active);
                                                    });
                                                }
                                            });
                                        });

                                    let new_tab_button = Self::tab_strip_button(ui, "New tab");
                                    new_tab_button.widget_info(|| {
                                        let mut info = WidgetInfo::new(WidgetType::Button);
                                        info.label = Some("New tab".into());
                                        info
                                    });
                                    if new_tab_button.clicked() {
                                        window.queue_user_interface_command(
                                            UserInterfaceCommand::NewWebView,
                                        );
                                    }

                                    // Remaining empty space is draggable and double-clickable
                                    let remaining_size = ui.available_size();
                                    if remaining_size.x > 0.0 {
                                        let drag_resp = ui.allocate_response(
                                            remaining_size,
                                            egui::Sense::click_and_drag(),
                                        );
                                        if drag_resp.drag_started_by(egui::PointerButton::Primary) {
                                            let _ = winit_window.drag_window();
                                        }
                                        if drag_resp.double_clicked_by(egui::PointerButton::Primary) {
                                            winit_window.set_maximized(!winit_window.is_maximized());
                                        }
                                    }
                                },
                            );
                        },
                    );
                });

                // 2. The Navigation / Address Bar (below the tabs)
                let frame = egui::Frame::default()
                    .fill(TOOLBAR_COLOR)
                    .inner_margin(Margin {
                        left: 10,
                        right: 10,
                        top: 5,
                        bottom: 6,
                    });
                let toolbar_panel = Panel::top("toolbar")
                    .show_separator_line(false)
                    .frame(frame)
                    .show_inside(ctx, |ui| {
                    ui.allocate_ui_with_layout(
                        ui.available_size(),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);

                            let back_button =
                                Self::nav_button(ui, NavIcon::Back, self.can_go_back, "Back");
                            back_button.widget_info(|| {
                                let mut info = WidgetInfo::new(WidgetType::Button);
                                info.label = Some("Back".into());
                                info
                            });
                            if back_button.clicked() {
                                *location_dirty = false;
                                window.queue_user_interface_command(UserInterfaceCommand::Back);
                            }

                            let forward_button =
                                Self::nav_button(ui, NavIcon::Forward, self.can_go_forward, "Forward");
                            forward_button.widget_info(|| {
                                let mut info = WidgetInfo::new(WidgetType::Button);
                                info.label = Some("Forward".into());
                                info
                            });
                            if forward_button.clicked() {
                                *location_dirty = false;
                                window.queue_user_interface_command(UserInterfaceCommand::Forward);
                            }

                            match self.load_status {
                                LoadStatus::Started | LoadStatus::HeadParsed => {
                                    let stop_button = Self::nav_button(ui, NavIcon::Stop, true, "Stop");
                                    stop_button.widget_info(|| {
                                        let mut info = WidgetInfo::new(WidgetType::Button);
                                        info.label = Some("Stop".into());
                                        info
                                    });
                                    if stop_button.clicked() {
                                        warn!("Do not support stop yet.");
                                    }
                                },
                                LoadStatus::Complete => {
                                    let reload_button = Self::nav_button(ui, NavIcon::Reload, true, "Reload");
                                    reload_button.widget_info(|| {
                                        let mut info = WidgetInfo::new(WidgetType::Button);
                                        info.label = Some("Reload".into());
                                        info
                                    });
                                    if reload_button.clicked() {
                                        *location_dirty = false;
                                        window.queue_user_interface_command(
                                            UserInterfaceCommand::Reload,
                                        );
                                    }
                                },
                            }

                            ui.add_space(8.0);

                            // The omnibox occupies the remaining width minus the preferences button (28px + 8px space)
                            let omnibox_width = (ui.available_width() - 36.0).max(100.0);
                            let omnibox_height = 30.0;

                            let location_id = egui::Id::new("location_input");
                            let is_focused = ui.memory(|m| m.focused() == Some(location_id));

                            // Fixed size omnibox allocation: guarantees no geometry changes or jumps when focused
                            let (omnibox_rect, omnibox_resp) = ui.allocate_exact_size(
                                Vec2::new(omnibox_width, omnibox_height),
                                egui::Sense::hover(),
                            );
                            let is_hovered = omnibox_resp.hovered();

                            let omnibox_bg = if is_focused {
                                Color32::from_rgb(32, 33, 37)
                            } else if is_hovered {
                                Color32::from_rgb(38, 39, 44)
                            } else {
                                Color32::from_rgb(32, 33, 37)
                            };

                            // A restrained radius keeps the address field aligned with the
                            // square-edged browser chrome instead of looking like a pill.
                            let corner_radius = 7.0;
                            ui.painter().rect_filled(omnibox_rect, corner_radius, omnibox_bg);

                            // Draw border stroke strictly INSIDE the fixed rect, so it NEVER expands the widget bounds
                            let omnibox_stroke = if is_focused {
                                Stroke::new(1.5_f32, Color32::from_rgb(88, 140, 240))
                            } else if is_hovered {
                                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 45))
                            } else {
                                Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 25),
                                )
                            };
                            ui.painter().rect_stroke(omnibox_rect, corner_radius, omnibox_stroke, StrokeKind::Inside);

                            // Vector lock or search icon on the left
                            let is_secure = location.starts_with("https://");
                            let is_http = location.starts_with("http://");
                            let ic = egui::pos2(
                                omnibox_rect.left() + 18.0,
                                if is_secure { omnibox_rect.center().y - 0.5 } else { omnibox_rect.center().y },
                            );
                            let icon_color = if is_focused {
                                Color32::from_rgb(88, 140, 240)
                            } else {
                                Color32::from_gray(160)
                            };
                            let stroke = Stroke::new(1.3_f32, icon_color);

                            let glyph = if is_secure { "\u{e72e}" } else if is_http { "\u{e774}" } else { "\u{e721}" };
                            if !Self::fluent_icon(ui, ic, glyph, 14.0, icon_color) {
                            if is_secure {
                                let body = egui::Rect::from_center_size(
                                    ic + Vec2::new(0.0, 2.0),
                                    Vec2::new(9.0, 7.0),
                                );
                                ui.painter().rect_stroke(
                                    body,
                                    1.5,
                                    stroke,
                                    StrokeKind::Inside,
                                );
                                let arm = 2.5;
                                let top_y = ic.y - 4.5;
                                let bot_y = body.top();
                                ui.painter().line_segment(
                                    [
                                        pos2(ic.x - arm, bot_y),
                                        pos2(ic.x - arm, top_y + 1.5),
                                    ],
                                    stroke,
                                );
                                ui.painter().line_segment(
                                    [
                                        pos2(ic.x + arm, bot_y),
                                        pos2(ic.x + arm, top_y + 1.5),
                                    ],
                                    stroke,
                                );
                                ui.painter().line_segment(
                                    [
                                        pos2(ic.x - arm, top_y + 1.5),
                                        pos2(ic.x - arm + 1.2, top_y),
                                    ],
                                    stroke,
                                );
                                ui.painter().line_segment(
                                    [
                                        pos2(ic.x + arm, top_y + 1.5),
                                        pos2(ic.x + arm - 1.2, top_y),
                                    ],
                                    stroke,
                                );
                                ui.painter().line_segment(
                                    [
                                        pos2(ic.x - arm + 1.2, top_y),
                                        pos2(ic.x + arm - 1.2, top_y),
                                    ],
                                    stroke,
                                );
                            } else if is_http {
                                let body = egui::Rect::from_center_size(
                                    ic + Vec2::new(0.0, 2.0),
                                    Vec2::new(9.0, 7.0),
                                );
                                ui.painter().rect_stroke(
                                    body,
                                    1.5,
                                    stroke,
                                    StrokeKind::Inside,
                                );
                                let arm = 2.5;
                                let top_y = ic.y - 5.5;
                                ui.painter().line_segment(
                                    [
                                        pos2(ic.x - arm, body.top()),
                                        pos2(ic.x - arm, top_y + 1.5),
                                    ],
                                    stroke,
                                );
                                ui.painter().line_segment(
                                    [
                                        pos2(ic.x - arm, top_y + 1.5),
                                        pos2(ic.x + arm, top_y + 0.5),
                                    ],
                                    stroke,
                                );
                            } else {
                                let glass_center = ic + Vec2::new(-1.0, -0.8);
                                ui.painter().circle_stroke(
                                    glass_center,
                                    3.2,
                                    Stroke::new(1.2_f32, icon_color),
                                );
                                ui.painter().line_segment(
                                    [
                                        glass_center + Vec2::new(2.3, 2.3),
                                        glass_center + Vec2::new(4.8, 4.8),
                                    ],
                                    Stroke::new(1.4_f32, icon_color),
                                );
                            }
                            }

                            // Stylize the text cursor (slim, modern blue caret) and selection
                            let visuals = ui.visuals_mut();
                            visuals.text_cursor.stroke = Stroke::new(
                                1.3_f32,
                                Color32::from_rgb(88, 140, 240),
                            );
                            visuals.selection.bg_fill =
                                Color32::from_rgba_unmultiplied(88, 140, 240, 80);

                            // Text edit region spanning full vertical height for perfect vertical centering
                            let text_rect = egui::Rect::from_min_max(
                                pos2(omnibox_rect.left() + 32.0, omnibox_rect.top()),
                                pos2(omnibox_rect.right() - 12.0, omnibox_rect.bottom()),
                            );
                            let mut child_ui = ui.new_child(
                                UiBuilder::new()
                                    .max_rect(text_rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                            );
                            let location_field = child_ui.add_sized(
                                text_rect.size(),
                                egui::TextEdit::singleline(location)
                                    .id(location_id)
                                    .frame(egui::Frame::NONE)
                                    .margin(egui::Margin::ZERO)
                                    .vertical_align(egui::Align::Center)
                                    .font(FontId::proportional(13.0))
                                    .text_color(Color32::from_rgb(240, 240, 245))
                                    .hint_text("Search Google or enter address"),
                            );

                            // Clicking anywhere on the omnibox pill requests focus on the field
                            if is_hovered && ui.input(|i| i.pointer.primary_clicked()) && !is_focused {
                                location_field.request_focus();
                            }

                            ui.add_space(8.0);

                            // Preferences button on the right
                            let experimental_preferences_enabled =
                                state.experimental_preferences_enabled();

                            let (prefs_rect, prefs_resp) = ui.allocate_exact_size(
                                Vec2::new(28.0, 28.0),
                                egui::Sense::click(),
                            );
                            let prefs_hovered = prefs_resp.hovered();
                            let prefs_active = prefs_resp.is_pointer_button_down_on();
                            let prefs_bg = if experimental_preferences_enabled {
                                Color32::from_rgb(220, 140, 20)
                            } else if prefs_active {
                                Color32::from_rgba_unmultiplied(255, 255, 255, 45)
                            } else if prefs_hovered {
                                Color32::from_rgba_unmultiplied(255, 255, 255, 22)
                            } else {
                                Color32::TRANSPARENT
                            };
                            ui.painter().rect_filled(prefs_rect, 14.0, prefs_bg);
                            let prefs_fg = if experimental_preferences_enabled {
                                Color32::BLACK
                            } else if prefs_hovered {
                                Color32::WHITE
                            } else {
                                Color32::from_gray(190)
                            };
                            if !Self::fluent_icon(ui, prefs_rect.center(), "\u{f196}", 16.0, prefs_fg) {
                                // A flask also identifies the experimental toggle without a system font.
                                let center = prefs_rect.center();
                                let points = [(-2.0, -6.0), (-2.0, -1.0), (-6.0, 6.0),
                                              (6.0, 6.0), (2.0, -1.0), (2.0, -6.0)];
                                ui.painter().add(egui::Shape::closed_line(
                                    points.into_iter().map(|(x, y)| center + Vec2::new(x, y)).collect(),
                                    Stroke::new(1.3_f32, prefs_fg),
                                ));
                            }
                            let prefs_resp = prefs_resp
                                .on_hover_text("Enable experimental preferences");
                            prefs_resp.widget_info(|| {
                                let mut info = WidgetInfo::new(WidgetType::Button);
                                info.label =
                                    Some("Enable experimental preferences".into());
                                info.selected = Some(experimental_preferences_enabled);
                                info
                            });
                            if prefs_resp.clicked() {
                                state.set_experimental_preferences_enabled(
                                    !experimental_preferences_enabled,
                                );
                                *location_dirty = false;
                                window.queue_user_interface_command(
                                    UserInterfaceCommand::ReloadAll,
                                );
                            }

                            if location_field.changed() {
                                *location_dirty = true;
                            }
                            // Handle address bar shortcut.
                            if ui.input(|i| {
                                if cfg!(target_os = "macos") {
                                    i.clone().consume_key(Modifiers::COMMAND, Key::L)
                                } else {
                                    i.clone().consume_key(Modifiers::COMMAND, Key::L) ||
                                        i.clone().consume_key(Modifiers::ALT, Key::D)
                                }
                            }) {
                                location_field.request_focus();
                            }
                            // Select address bar text when it's focused (click or shortcut).
                            if location_field.gained_focus() &&
                                let Some(mut state) =
                                    TextEditState::load(ui.ctx(), location_id)
                            {
                                state.cursor.set_char_range(Some(CCursorRange::two(
                                    CCursor::new(0),
                                    CCursor::new(location.len()),
                                )));
                                state.store(ui.ctx(), location_id);
                            }
                            // Navigate to address when enter is pressed in the address bar.
                            if location_field.lost_focus() &&
                                ui.input(|i| i.clone().key_pressed(Key::Enter))
                            {
                                window.queue_user_interface_command(
                                    UserInterfaceCommand::Go(location.clone()),
                                );
                            }
                        },
                    );
                });

                *toolbar_height = Length::new(toolbar_panel.response.rect.max.y);
            } else {
                *toolbar_height = Length::default();
            }

            let scale =
                Scale::<_, DeviceIndependentPixel, DevicePixel>::new(ctx.pixels_per_point());

            headed_window.for_each_active_dialog(window, |dialog| dialog.update(ctx));

            // If the top parts of the GUI changed size, then update the size of the WebView and also
            // the size of its RenderingContext.
            let available_rect = ctx.available_rect_before_wrap();

            // Build a graft node for each WebView.
            let affine = {
                // The grafted WebView tree reports bounds in device pixels relative to the
                // WebView's own origin, so this node supplies the offset of the WebView within
                // the window, and scales it to the same scale as the rest of the nodes in egui's
                // AccessKit tree.
                let scale = (1.0 / window.platform_window().hidpi_scale_factor().get()) as f64;
                let x = available_rect.min.x as f64;
                let y = available_rect.min.y as f64;
                Affine::new([scale, 0.0, 0.0, scale, x, y])
            };
            for (webview_id, webview) in window.webviews() {
                if let Some(tree_id) = webview.accesskit_tree_id() {
                    let id = egui::Id::new(webview_id);
                    ctx.accesskit_node_builder(id, |node| {
                        node.set_tree_id(tree_id);
                        // Only the transform is set: AccessKit consumers exclude graft nodes from
                        // the presented tree, so bounds on this node would never be read.
                        node.set_transform(affine);
                    });
                }
            }
            let size = Size2D::new(available_rect.width(), available_rect.height()) * scale;
            if let Some(webview) = window.active_webview() &&
                size != webview.size()
            {
                // `rect` is sized to just the WebView viewport, which is required by
                // `OffscreenRenderingContext` See:
                // <https://github.com/servo/servo/issues/38369#issuecomment-3138378527>
                webview.resize(PhysicalSize::new(size.width as u32, size.height as u32))
            }

            if let Some(status_text) = &self.status_text {
                egui::Tooltip::always_open(
                    ctx.clone(),
                    LayerId::new(Order::Tooltip, Id::new("tooltip")),
                    "tooltip layer".into(),
                    pos2(0.0, available_rect.max.y),
                )
                .show(|ui| ui.add(Label::new(status_text.clone()).extend()));
            }

            window.repaint_webviews();

            if let Some(render_to_parent) = rendering_context.render_to_parent_callback() {
                ctx.layer_painter(LayerId::background()).add(PaintCallback {
                    rect: available_rect,
                    callback: Arc::new(CallbackFn::new(move |info, painter| {
                        let clip = info.viewport_in_pixels();
                        let rect_in_parent = Rect::new(
                            Point2D::new(clip.left_px, clip.from_bottom_px),
                            Size2D::new(clip.width_px, clip.height_px),
                        );
                        render_to_parent(painter.gl(), rect_in_parent)
                    })),
                });
            }
        });

        // If any egui widget requested a repaint, also request a repaint for our
        // containing window. This allows egui widget to animate on their own.
        if self.context.egui_ctx.has_requested_repaint() {
            window.set_needs_repaint();
        }

        let adapter = self
            .context
            .egui_winit
            .accesskit
            .as_mut()
            .expect("guaranteed by Gui::new()");
        for tree_update in self.pending_accesskit_updates.drain(..) {
            adapter.update_if_active(|| tree_update);
        }
    }

    /// Paint the GUI, as of the last update.
    pub(crate) fn paint(&mut self, window: &Window) {
        self.rendering_context
            .make_current()
            .expect("Could not make RenderingContext current");
        self.rendering_context
            .parent_context()
            .prepare_for_rendering();
        self.context.paint(window);
        self.rendering_context.parent_context().present();
    }

    /// Updates the location field from the given [`RunningAppState`], unless the user has started
    /// editing it without clicking Go, returning true iff it has changed (needing an egui update).
    fn update_location_in_toolbar(&mut self, window: &ServoShellWindow) -> bool {
        // User edited without clicking Go?
        if self.location_dirty {
            return false;
        }

        let current_url_string = window
            .active_webview()
            .and_then(|webview| Some(webview.url()?.to_string()));
        match current_url_string {
            Some(location) if location != self.location => {
                self.location = location;
                true
            },
            _ => false,
        }
    }

    fn update_load_status(&mut self, window: &ServoShellWindow) -> bool {
        let state_status = window
            .active_webview()
            .map(|webview| webview.load_status())
            .unwrap_or(LoadStatus::Complete);
        let old_status = std::mem::replace(&mut self.load_status, state_status);
        let status_changed = old_status != self.load_status;

        // When the load status changes, we want the new changes to the URL to start
        // being reflected in the location bar.
        if status_changed {
            self.location_dirty = false;
        }

        status_changed
    }

    fn update_status_text(&mut self, window: &ServoShellWindow) -> bool {
        let state_status = window
            .active_webview()
            .and_then(|webview| webview.status_text());
        let old_status = std::mem::replace(&mut self.status_text, state_status);
        old_status != self.status_text
    }

    fn update_can_go_back_and_forward(&mut self, window: &ServoShellWindow) -> bool {
        let (can_go_back, can_go_forward) = window
            .active_webview()
            .map(|webview| (webview.can_go_back(), webview.can_go_forward()))
            .unwrap_or((false, false));
        let old_can_go_back = std::mem::replace(&mut self.can_go_back, can_go_back);
        let old_can_go_forward = std::mem::replace(&mut self.can_go_forward, can_go_forward);
        old_can_go_back != self.can_go_back || old_can_go_forward != self.can_go_forward
    }

    /// Updates all fields taken from the given [`ServoShellWindow`], such as the location field.
    /// Returns true iff the egui needs an update.
    pub(crate) fn update_webview_data(&mut self, window: &ServoShellWindow) -> bool {
        // Note: We must use the "bitwise OR" (|) operator here instead of "logical OR" (||)
        //       because logical OR would short-circuit if any of the functions return true.
        //       We want to ensure that all functions are called. The "bitwise OR" operator
        //       does not short-circuit.
        self.update_load_status(window) |
            self.update_location_in_toolbar(window) |
            self.update_status_text(window) |
            self.update_can_go_back_and_forward(window)
    }

    /// Returns true if a redraw is required after handling the provided event.
    pub(crate) fn handle_accesskit_event(
        &mut self,
        event: &egui_winit::accesskit_winit::WindowEvent,
    ) -> bool {
        match event {
            egui_winit::accesskit_winit::WindowEvent::InitialTreeRequested => {
                self.context.egui_ctx.enable_accesskit();
                true
            },
            egui_winit::accesskit_winit::WindowEvent::ActionRequested(req) => {
                self.context
                    .egui_winit
                    .on_accesskit_action_request(req.clone());
                true
            },
            egui_winit::accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                self.context.egui_ctx.disable_accesskit();
                false
            },
        }
    }

    pub(crate) fn set_zoom_factor(&self, factor: f32) {
        self.context.egui_ctx.set_zoom_factor(factor);
    }

    pub(crate) fn notify_accessibility_tree_update(&mut self, tree_update: accesskit::TreeUpdate) {
        self.pending_accesskit_updates.push(tree_update);
    }
}

fn embedder_image_to_egui_image(image: &Image) -> egui::ColorImage {
    let width = image.width as usize;
    let height = image.height as usize;

    match image.format {
        PixelFormat::K8 => egui::ColorImage::from_gray([width, height], image.data()),
        PixelFormat::KA8 => {
            // Convert to rgba
            let data: Vec<u8> = image
                .data()
                .chunks_exact(2)
                .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
                .collect();
            egui::ColorImage::from_rgba_unmultiplied([width, height], &data)
        },
        PixelFormat::RGB8 => egui::ColorImage::from_rgb([width, height], image.data()),
        PixelFormat::RGBA8 => {
            egui::ColorImage::from_rgba_unmultiplied([width, height], image.data())
        },
        PixelFormat::BGRA8 => {
            // Convert from BGRA to RGBA
            let data: Vec<u8> = image
                .data()
                .chunks_exact(4)
                .flat_map(|chunk| [chunk[2], chunk[1], chunk[0], chunk[3]])
                .collect();
            egui::ColorImage::from_rgba_unmultiplied([width, height], &data)
        },
    }
}

/// Uploads all favicons that have not yet been processed to the GPU.
fn load_pending_favicons(
    ctx: &egui::Context,
    window: &ServoShellWindow,
    texture_cache: &mut HashMap<WebViewId, (egui::TextureHandle, egui::load::SizedTexture)>,
) {
    for id in window.take_pending_favicon_loads() {
        let Some(webview) = window.webview_by_id(id) else {
            continue;
        };
        let Some(favicon) = webview.favicon() else {
            continue;
        };

        let egui_image = embedder_image_to_egui_image(&favicon);
        let handle = ctx.load_texture(format!("favicon-{id:?}"), egui_image, Default::default());
        let texture = egui::load::SizedTexture::new(
            handle.id(),
            egui::vec2(favicon.width as f32, favicon.height as f32),
        );

        // We don't need the handle anymore but we can't drop it either since that would cause
        // the texture to be freed.
        texture_cache.insert(id, (handle, texture));
    }
}
