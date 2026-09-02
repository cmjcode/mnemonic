//! Liquid-Glass & Shapr3D theme tokens, frames, and palette styling.
//! Integrates `egui_icons` (Material Design & MDI vector icons) and supports
//! both Dark and Light theme modes.

use egui::{
    Color32, CornerRadius, FontId, Frame, Margin, Shadow, Stroke, Style, Vec2, Visuals,
};

// ─── Theme Constants & Color Tokens ──────────────────────────────────────────

pub const MIN_TOUCH_TARGET: f32 = 28.0;
pub const BOTTOM_RIGHT_PANEL_WIDTH: f32 = 260.0;
pub const ICON_SIZE_DEFAULT: f32 = 18.0;

// Shapr3D & Liquid Glass Color Tokens
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(10, 132, 255); // #0a84ff
pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(255, 149, 0); // #ff9500 (Active highlight)
pub const ACCENT_GREEN: Color32 = Color32::from_rgb(48, 209, 88); // #30d158 (Success / Valid)
pub const ACCENT_PURPLE: Color32 = Color32::from_rgb(175, 82, 222); // #af52de (Selection)
pub const BG_CANVAS: Color32 = Color32::from_rgb(18, 19, 22); // Deep viewport background
pub const BG_PANEL_DARK: Color32 = Color32::from_rgba_premultiplied(16, 18, 22, 225); // ~88% translucent glass
pub const BG_CARD_DARK: Color32 = Color32::from_rgba_premultiplied(26, 30, 38, 220); // Card fill
pub const BG_HOVER_DARK: Color32 = Color32::from_rgba_premultiplied(40, 45, 56, 220); // Hover fill
pub const BORDER_SUBTLE: Color32 = Color32::from_rgba_premultiplied(65, 75, 95, 130); // Glass border
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(245, 245, 247);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(142, 142, 147);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(99, 99, 102);

// Liquid Glass Palette Aliases (Backward-compatibility)
pub const GLASS_BG: Color32 = BG_CANVAS;
pub const GLASS_SURFACE: Color32 = BG_CARD_DARK;
pub const GLASS_SURFACE_HIGH: Color32 = BG_PANEL_DARK;
pub const GLASS_SIDEBAR: Color32 = BG_PANEL_DARK;
pub const GLASS_TOPBAR: Color32 = BG_PANEL_DARK;
pub const GLASS_ACCENT: Color32 = ACCENT_BLUE;
pub const GLASS_ACCENT_HOVER: Color32 = Color32::from_rgb(64, 156, 255);
pub const GLASS_ACCENT_ACTIVE: Color32 = Color32::from_rgb(0, 110, 220);
pub const GLASS_BORDER: Color32 = BORDER_SUBTLE;
pub const GLASS_BORDER_HOVER: Color32 = Color32::from_rgba_premultiplied(10, 132, 255, 140);
pub const GLASS_TEXT_PRIMARY: Color32 = TEXT_PRIMARY;
pub const GLASS_TEXT_SECONDARY: Color32 = TEXT_SECONDARY;
pub const GLASS_TEXT_FAINT: Color32 = TEXT_MUTED;
pub const GLASS_ERROR: Color32 = Color32::from_rgb(239, 68, 68);
pub const GLASS_SEPARATOR: Color32 = Color32::from_rgba_premultiplied(148, 163, 184, 35);

// Layout Constants
pub const ROUNDING_SM: u8 = 6;
pub const ROUNDING_MD: u8 = 10;
pub const ROUNDING_LG: u8 = 14;
pub const ROUNDING_XL: u8 = 20;

pub const CARD_MIN_WIDTH: f32 = 220.0;
pub const GRID_GAP: f32 = 12.0;
pub const SIDEBAR_WIDTH: f32 = 280.0;
pub const CHAT_SIDEBAR_WIDTH: f32 = 380.0;
pub const TOPBAR_HEIGHT: f32 = 44.0;

// ─── Theme Mode ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    Light,
    #[default]
    Dark,
}

impl ThemeMode {
    pub fn toggled(self) -> Self {
        match self {
            ThemeMode::Light => ThemeMode::Dark,
            ThemeMode::Dark => ThemeMode::Light,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Light => "☀ Terang",
            ThemeMode::Dark => "🌙 Gelap",
        }
    }

    fn visuals(self) -> Visuals {
        match self {
            ThemeMode::Dark => {
                let mut v = Visuals::dark();
                v.panel_fill = BG_PANEL_DARK;
                v.window_fill = BG_PANEL_DARK;
                v.faint_bg_color = BG_CARD_DARK;
                v.extreme_bg_color = Color32::from_rgb(12, 13, 15);
                v.window_stroke = Stroke::new(1.0, BORDER_SUBTLE);
                v.window_corner_radius = CornerRadius::same(ROUNDING_MD);
                v.menu_corner_radius = CornerRadius::same(ROUNDING_SM);

                v.widgets.noninteractive.bg_fill = BG_PANEL_DARK;
                v.widgets.noninteractive.weak_bg_fill = BG_PANEL_DARK;
                v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, GLASS_SEPARATOR);
                v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_SECONDARY);
                v.widgets.noninteractive.corner_radius = CornerRadius::same(ROUNDING_SM);

                v.widgets.inactive.bg_fill = Color32::from_rgba_premultiplied(28, 30, 36, 140);
                v.widgets.inactive.weak_bg_fill = Color32::from_rgba_premultiplied(28, 30, 36, 140);
                v.widgets.inactive.corner_radius = CornerRadius::same(ROUNDING_SM);
                v.widgets.inactive.bg_stroke = Stroke::new(0.5, BORDER_SUBTLE);
                v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);

                v.widgets.hovered.bg_fill = BG_HOVER_DARK;
                v.widgets.hovered.weak_bg_fill = BG_HOVER_DARK;
                v.widgets.hovered.corner_radius = CornerRadius::same(ROUNDING_SM);
                v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_BLUE);
                v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

                v.widgets.active.bg_fill = ACCENT_BLUE;
                v.widgets.active.weak_bg_fill = ACCENT_BLUE;
                v.widgets.active.corner_radius = CornerRadius::same(ROUNDING_SM);
                v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_BLUE);
                v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

                v.widgets.open.bg_fill = BG_CARD_DARK;
                v.widgets.open.weak_bg_fill = BG_CARD_DARK;
                v.widgets.open.corner_radius = CornerRadius::same(ROUNDING_SM);

                v.selection.bg_fill = Color32::from_rgba_premultiplied(10, 132, 255, 60);
                v.selection.stroke = Stroke::new(1.0, ACCENT_BLUE);
                v.hyperlink_color = GLASS_ACCENT_HOVER;
                v.override_text_color = Some(TEXT_PRIMARY);
                v
            }
            ThemeMode::Light => {
                let mut v = Visuals::light();
                v.window_corner_radius = CornerRadius::same(ROUNDING_MD);
                v.menu_corner_radius = CornerRadius::same(ROUNDING_SM);
                v.widgets.inactive.corner_radius = CornerRadius::same(ROUNDING_SM);
                v.widgets.hovered.corner_radius = CornerRadius::same(ROUNDING_SM);
                v.widgets.active.corner_radius = CornerRadius::same(ROUNDING_SM);
                v.selection.bg_fill = Color32::from_rgba_premultiplied(10, 132, 255, 50);
                v.selection.stroke = Stroke::new(1.0, ACCENT_BLUE);
                v
            }
        }
    }
}

// ─── Frame Helpers ────────────────────────────────────────────────────────────

/// Helper frame glassmorphism untuk panel mengambang.
pub fn glass_frame() -> Frame {
    Frame {
        inner_margin: Margin::symmetric(10, 6),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::same(ROUNDING_MD),
        shadow: Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(80),
        },
        fill: BG_PANEL_DARK,
        stroke: Stroke::new(1.0, BORDER_SUBTLE),
    }
}

/// Helper frame untuk kartu-kartu catatan / outliner.
pub fn card_frame() -> Frame {
    Frame {
        inner_margin: Margin::same(10),
        outer_margin: Margin::symmetric(0, 2),
        corner_radius: CornerRadius::same(ROUNDING_MD),
        shadow: Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(60),
        },
        fill: BG_CARD_DARK,
        stroke: Stroke::new(0.5, BORDER_SUBTLE),
    }
}

/// Helper frame untuk kapsul / pill mengambang (mis. status bar, tag chip, zoom pill).
pub fn pill_frame() -> Frame {
    Frame {
        inner_margin: Margin::symmetric(10, 5),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::same(ROUNDING_LG),
        shadow: Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(70),
        },
        fill: BG_PANEL_DARK,
        stroke: Stroke::new(1.0, BORDER_SUBTLE),
    }
}

/// Helper frame untuk badge putih kontras tinggi di kanvas.
pub fn dimension_pill_frame() -> Frame {
    Frame {
        inner_margin: Margin::symmetric(8, 4),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::same(ROUNDING_SM),
        shadow: Shadow {
            offset: [0, 2],
            blur: 6,
            spread: 0,
            color: Color32::from_black_alpha(120),
        },
        fill: Color32::from_rgba_premultiplied(240, 242, 245, 245),
        stroke: Stroke::new(1.0, Color32::from_gray(180)),
    }
}

/// Standard glass `egui::Frame` for note/PDF cards.
pub fn glass_card_frame() -> Frame {
    card_frame()
}

/// Frame untuk sidebar kiri tetap (Fixed Left Side Panel).
pub fn fixed_sidebar_frame() -> Frame {
    Frame {
        inner_margin: Margin::symmetric(8, 6),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::ZERO,
        shadow: Shadow::NONE,
        fill: BG_PANEL_DARK,
        stroke: Stroke::new(1.0, BORDER_SUBTLE),
    }
}

/// Glass frame for elevated overlay panels (sidebar).
pub fn glass_panel_frame() -> Frame {
    Frame {
        inner_margin: Margin::ZERO,
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::same(ROUNDING_MD),
        shadow: Shadow {
            offset: [4, 0],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(150),
        },
        fill: BG_PANEL_DARK,
        stroke: Stroke::new(1.0, BORDER_SUBTLE),
    }
}

/// Glass frame for the floating top bar.
pub fn glass_topbar_frame() -> Frame {
    Frame {
        inner_margin: Margin::symmetric(14, 6),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::same(ROUNDING_MD),
        shadow: Shadow {
            offset: [0, 6],
            blur: 20,
            spread: 0,
            color: Color32::from_black_alpha(140),
        },
        fill: Color32::from_rgba_premultiplied(20, 24, 30, 235),
        stroke: Stroke::new(1.0, Color32::from_rgba_premultiplied(80, 95, 120, 100)),
    }
}

/// Compact pill-style frame for tag chips.
pub fn tag_chip_frame(color: Color32) -> Frame {
    let fill = Color32::from_rgba_premultiplied(
        (color.r() as u16 * 35 / 255) as u8,
        (color.g() as u16 * 35 / 255) as u8,
        (color.b() as u16 * 35 / 255) as u8,
        60,
    );
    Frame {
        inner_margin: Margin::symmetric(8, 3),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::same(ROUNDING_LG),
        shadow: Shadow::NONE,
        fill,
        stroke: Stroke::new(
            1.0,
            Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), 100),
        ),
    }
}

// ─── Card Colour Palette ──────────────────────────────────────────────────────

pub const PALETTE: &[(&str, Color32)] = &[
    ("yellow", Color32::from_rgba_premultiplied(80, 70, 5, 50)),
    ("green", Color32::from_rgba_premultiplied(10, 70, 45, 50)),
    ("blue", Color32::from_rgba_premultiplied(20, 50, 100, 50)),
    ("purple", Color32::from_rgba_premultiplied(60, 30, 100, 50)),
    ("pink", Color32::from_rgba_premultiplied(90, 20, 60, 50)),
    ("red", Color32::from_rgba_premultiplied(90, 15, 15, 50)),
    ("orange", Color32::from_rgba_premultiplied(90, 45, 5, 50)),
    ("teal", Color32::from_rgba_premultiplied(5, 75, 70, 50)),
    ("gray", Color32::from_rgba_premultiplied(40, 45, 60, 50)),
];

pub const PALETTE_SOLID: &[(&str, Color32)] = &[
    ("yellow", Color32::from_rgb(234, 179, 8)),
    ("green", Color32::from_rgb(16, 185, 129)),
    ("blue", Color32::from_rgb(59, 130, 246)),
    ("purple", Color32::from_rgb(139, 92, 246)),
    ("pink", Color32::from_rgb(236, 72, 153)),
    ("red", Color32::from_rgb(239, 68, 68)),
    ("orange", Color32::from_rgb(249, 115, 22)),
    ("teal", Color32::from_rgb(20, 184, 166)),
    ("gray", Color32::from_rgb(100, 116, 139)),
];

pub fn color_for(name: Option<&str>) -> Option<Color32> {
    let name = name?;
    PALETTE.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

pub fn color_solid_for(name: Option<&str>) -> Option<Color32> {
    let name = name?;
    PALETTE_SOLID
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
}

// ─── Tag Colours ──────────────────────────────────────────────────────────────

const TAG_COLORS: &[Color32] = &[
    Color32::from_rgb(10, 132, 255), // blue
    Color32::from_rgb(48, 209, 88),  // green
    Color32::from_rgb(175, 82, 222), // purple
    Color32::from_rgb(255, 149, 0),  // orange
    Color32::from_rgb(255, 59, 48),  // red
    Color32::from_rgb(100, 210, 255),// teal/cyan
    Color32::from_rgb(255, 45, 85),  // pink
    Color32::from_rgb(255, 214, 10), // yellow
];

pub fn tag_color(tag: &str) -> Color32 {
    let mut hash: u32 = 2166136261;
    for b in tag.to_lowercase().as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    TAG_COLORS[(hash as usize) % TAG_COLORS.len()]
}

// ─── Apply Theme to Context ──────────────────────────────────────────────────

/// Apply ThemeMode and touch-target style to egui context.
pub fn apply_theme(ctx: &egui::Context, mode: ThemeMode) {
    egui_icons::initialize(ctx);

    let theme = match mode {
        ThemeMode::Dark => egui::Theme::Dark,
        ThemeMode::Light => egui::Theme::Light,
    };
    ctx.set_theme(theme);

    let mut style = Style {
        visuals: mode.visuals(),
        ..Default::default()
    };
    style.spacing.interact_size.y = MIN_TOUCH_TARGET;
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.window_margin = Margin::same(12);
    style.spacing.scroll.bar_width = 6.0;

    use egui::TextStyle::*;
    style.text_styles.insert(Small, FontId::proportional(11.5));
    style.text_styles.insert(Body, FontId::proportional(13.5));
    style.text_styles.insert(Button, FontId::proportional(13.0));
    style.text_styles.insert(Monospace, FontId::monospace(13.0));
    style.text_styles.insert(Heading, FontId::proportional(17.0));

    ctx.set_style_of(theme, style);
}

/// Backward compatibility: apply default dark theme.
pub fn apply_liquid_glass_theme(ctx: &egui::Context) {
    apply_theme(ctx, ThemeMode::Dark);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_for_known_and_unknown_names() {
        assert!(color_for(Some("yellow")).is_some());
        assert_eq!(color_for(Some("not-a-color")), None);
        assert_eq!(color_for(None), None);
    }

    #[test]
    fn tag_color_is_deterministic() {
        assert_eq!(tag_color("rumah"), tag_color("rumah"));
        assert_eq!(tag_color("Rumah"), tag_color("rumah")); // case-insensitive
    }

    #[test]
    fn test_theme_apply_and_icons() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, ThemeMode::Dark);
        assert_eq!(ThemeMode::Dark.toggled(), ThemeMode::Light);
    }
}
