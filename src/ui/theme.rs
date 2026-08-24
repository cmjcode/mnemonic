//! Liquid-Glass theme constants, card colour palette (§3.1.2: "8-12 warna
//! pastel ala Keep"), and tag-colour assignment (§3.1.3).  Kept separate
//! from `notes::` so that module stays free of `egui`. Callers: `app.rs`.

use egui::{Color32, CornerRadius, FontId, Shadow, Stroke, Vec2, Visuals};

// ─── Liquid Glass Palette ────────────────────────────────────────────────────

/// Base background — very dark indigo, almost black.
pub const GLASS_BG: Color32 = Color32::from_rgb(10, 10, 18);

/// Primary surface for panels/cards — dark with slight blue tint.
pub const GLASS_SURFACE: Color32 = Color32::from_rgba_premultiplied(22, 24, 38, 240);

/// Slightly lighter surface for elevated cards/popups.
pub const GLASS_SURFACE_HIGH: Color32 = Color32::from_rgba_premultiplied(32, 35, 55, 245);

/// Sidebar overlay background.
pub const GLASS_SIDEBAR: Color32 = Color32::from_rgba_premultiplied(15, 17, 30, 235);

/// Top bar background — subtle glass effect.
pub const GLASS_TOPBAR: Color32 = Color32::from_rgba_premultiplied(12, 14, 24, 230);

/// Accent colour — electric indigo/blue.
pub const GLASS_ACCENT: Color32 = Color32::from_rgb(99, 102, 241);

/// Lighter accent for hover.
pub const GLASS_ACCENT_HOVER: Color32 = Color32::from_rgb(129, 140, 248);

/// Accent for active/selected state.
pub const GLASS_ACCENT_ACTIVE: Color32 = Color32::from_rgb(79, 70, 229);

/// Thin glowing border colour.
pub const GLASS_BORDER: Color32 = Color32::from_rgba_premultiplied(99, 102, 241, 60);

/// Slightly brighter border for cards on hover.
pub const GLASS_BORDER_HOVER: Color32 = Color32::from_rgba_premultiplied(129, 140, 248, 120);

/// Primary text — near white.
pub const GLASS_TEXT_PRIMARY: Color32 = Color32::from_rgb(226, 232, 240);

/// Secondary text — muted slate.
pub const GLASS_TEXT_SECONDARY: Color32 = Color32::from_rgb(148, 163, 184);

/// Faint text for timestamps, counts, etc.
pub const GLASS_TEXT_FAINT: Color32 = Color32::from_rgb(100, 116, 139);

/// Red for destructive actions / error state.
pub const GLASS_ERROR: Color32 = Color32::from_rgb(239, 68, 68);

/// Subtle separator line.
pub const GLASS_SEPARATOR: Color32 = Color32::from_rgba_premultiplied(148, 163, 184, 25);

// ─── Layout constants ─────────────────────────────────────────────────────────

pub const ROUNDING_SM: u8 = 8;
pub const ROUNDING_MD: u8 = 12;
pub const ROUNDING_LG: u8 = 16;
pub const ROUNDING_XL: u8 = 24;

pub const CARD_MIN_WIDTH: f32 = 200.0;
pub const GRID_GAP: f32 = 12.0;
pub const SIDEBAR_WIDTH: f32 = 220.0;
pub const TOPBAR_HEIGHT: f32 = 52.0;

// ─── Card colour palette ──────────────────────────────────────────────────────

/// `(frontmatter value, display color)` pairs — semi-transparent tones
/// that look great on the dark glass background.
/// An unrecognised or absent value falls back to the default card surface
/// (`color_for` returns `None`).
pub const PALETTE: &[(&str, Color32)] = &[
    ("yellow", Color32::from_rgba_premultiplied(80, 70, 5, 50)),
    ("green",  Color32::from_rgba_premultiplied(10, 70, 45, 50)),
    ("blue",   Color32::from_rgba_premultiplied(20, 50, 100, 50)),
    ("purple", Color32::from_rgba_premultiplied(60, 30, 100, 50)),
    ("pink",   Color32::from_rgba_premultiplied(90, 20, 60, 50)),
    ("red",    Color32::from_rgba_premultiplied(90, 15, 15, 50)),
    ("orange", Color32::from_rgba_premultiplied(90, 45, 5, 50)),
    ("teal",   Color32::from_rgba_premultiplied(5, 75, 70, 50)),
    ("gray",   Color32::from_rgba_premultiplied(40, 45, 60, 50)),
];

/// Solid accent versions of the palette colours (for badges, chips, etc.)
pub const PALETTE_SOLID: &[(&str, Color32)] = &[
    ("yellow", Color32::from_rgb(234, 179, 8)),
    ("green",  Color32::from_rgb(16, 185, 129)),
    ("blue",   Color32::from_rgb(59, 130, 246)),
    ("purple", Color32::from_rgb(139, 92, 246)),
    ("pink",   Color32::from_rgb(236, 72, 153)),
    ("red",    Color32::from_rgb(239, 68, 68)),
    ("orange", Color32::from_rgb(249, 115, 22)),
    ("teal",   Color32::from_rgb(20, 184, 166)),
    ("gray",   Color32::from_rgb(100, 116, 139)),
];

/// The display colour for a note's `frontmatter.color` value, or `None` if
/// unset/unrecognised (caller should use the default card styling).
pub fn color_for(name: Option<&str>) -> Option<Color32> {
    let name = name?;
    PALETTE.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

/// Solid version of the card colour (for borders / badges).
pub fn color_solid_for(name: Option<&str>) -> Option<Color32> {
    let name = name?;
    PALETTE_SOLID.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

// ─── Tag colours ──────────────────────────────────────────────────────────────

const TAG_COLORS: &[Color32] = &[
    Color32::from_rgb(99, 102, 241),  // indigo
    Color32::from_rgb(16, 185, 129),  // emerald
    Color32::from_rgb(168, 85, 247),  // violet
    Color32::from_rgb(236, 72, 153),  // pink
    Color32::from_rgb(245, 158, 11),  // amber
    Color32::from_rgb(20, 184, 166),  // teal
    Color32::from_rgb(59, 130, 246),  // blue
    Color32::from_rgb(239, 68, 68),   // red
];

/// A deterministic accent colour for a tag chip, picked by hashing the tag
/// name — approximates §3.1.3's "beri warna tag".
pub fn tag_color(tag: &str) -> Color32 {
    let mut hash: u32 = 2166136261;
    for b in tag.to_lowercase().as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    TAG_COLORS[(hash as usize) % TAG_COLORS.len()]
}

// ─── Liquid Glass Theme Application ──────────────────────────────────────────

/// Apply the Liquid Glass dark theme to the egui context. Call once per
/// frame in `eframe::App::ui` before any painting.
pub fn apply_liquid_glass_theme(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();

    visuals.window_fill = GLASS_SURFACE_HIGH;
    visuals.panel_fill = GLASS_BG;
    visuals.window_shadow = Shadow {
        offset: [0, 8],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    visuals.window_corner_radius = CornerRadius::same(ROUNDING_LG);
    visuals.window_stroke = Stroke::new(1.0, GLASS_BORDER);

    visuals.widgets.noninteractive.bg_fill = GLASS_SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = GLASS_SURFACE;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, GLASS_SEPARATOR);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, GLASS_TEXT_SECONDARY);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(ROUNDING_SM);

    visuals.widgets.inactive.bg_fill = Color32::from_rgba_premultiplied(99, 102, 241, 20);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgba_premultiplied(32, 35, 55, 200);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, GLASS_BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, GLASS_TEXT_PRIMARY);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(ROUNDING_SM);

    visuals.widgets.hovered.bg_fill = Color32::from_rgba_premultiplied(99, 102, 241, 40);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgba_premultiplied(45, 48, 72, 220);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, GLASS_BORDER_HOVER);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.5, GLASS_TEXT_PRIMARY);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(ROUNDING_SM);

    visuals.widgets.active.bg_fill = GLASS_ACCENT_ACTIVE;
    visuals.widgets.active.weak_bg_fill = GLASS_ACCENT_ACTIVE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, GLASS_ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
    visuals.widgets.active.corner_radius = CornerRadius::same(ROUNDING_SM);

    visuals.widgets.open.bg_fill = Color32::from_rgba_premultiplied(99, 102, 241, 50);
    visuals.widgets.open.weak_bg_fill = Color32::from_rgba_premultiplied(45, 48, 72, 230);
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, GLASS_ACCENT);
    visuals.widgets.open.fg_stroke = Stroke::new(1.5, GLASS_TEXT_PRIMARY);
    visuals.widgets.open.corner_radius = CornerRadius::same(ROUNDING_SM);

    visuals.selection.bg_fill = Color32::from_rgba_premultiplied(99, 102, 241, 70);
    visuals.selection.stroke = Stroke::new(1.0, GLASS_ACCENT);
    visuals.hyperlink_color = GLASS_ACCENT_HOVER;

    visuals.extreme_bg_color = Color32::from_rgb(6, 6, 12);
    visuals.code_bg_color = Color32::from_rgba_premultiplied(0, 0, 0, 80);
    visuals.warn_fg_color = Color32::from_rgb(251, 146, 60);
    visuals.error_fg_color = GLASS_ERROR;
    visuals.override_text_color = Some(GLASS_TEXT_PRIMARY);

    ctx.set_visuals(visuals);

    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.indent = 18.0;
        style.spacing.scroll.bar_width = 6.0;

        use egui::TextStyle::*;
        style.text_styles.insert(Small, FontId::proportional(11.0));
        style.text_styles.insert(Body, FontId::proportional(13.5));
        style.text_styles.insert(Button, FontId::proportional(13.5));
        style.text_styles.insert(Monospace, FontId::monospace(13.0));
        style.text_styles.insert(Heading, FontId::proportional(18.0));
    });
}

/// Standard glass `egui::Frame` for note/PDF cards.
pub fn glass_card_frame() -> egui::Frame {
    egui::Frame {
        inner_margin: egui::Margin::same(12),
        outer_margin: egui::Margin::ZERO,
        corner_radius: CornerRadius::same(ROUNDING_LG),
        shadow: Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(80),
        },
        fill: GLASS_SURFACE,
        stroke: Stroke::new(1.0, GLASS_BORDER),
    }
}

/// Glass frame for elevated overlay panels (sidebar).
pub fn glass_panel_frame() -> egui::Frame {
    egui::Frame {
        inner_margin: egui::Margin::ZERO,
        outer_margin: egui::Margin::ZERO,
        corner_radius: CornerRadius::same(ROUNDING_LG),
        shadow: Shadow {
            offset: [4, 0],
            blur: 32,
            spread: 0,
            color: Color32::from_black_alpha(160),
        },
        fill: GLASS_SIDEBAR,
        stroke: Stroke::new(1.0, GLASS_BORDER),
    }
}

/// Glass frame for the top bar.
pub fn glass_topbar_frame() -> egui::Frame {
    egui::Frame {
        inner_margin: egui::Margin::symmetric(16, 8),
        outer_margin: egui::Margin::ZERO,
        corner_radius: CornerRadius::ZERO,
        shadow: Shadow {
            offset: [0, 2],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(100),
        },
        fill: GLASS_TOPBAR,
        stroke: Stroke::new(0.0, Color32::TRANSPARENT),
    }
}

/// Compact pill-style frame for tag chips.
pub fn tag_chip_frame(color: Color32) -> egui::Frame {
    let fill = Color32::from_rgba_premultiplied(
        (color.r() as u16 * 30 / 255) as u8,
        (color.g() as u16 * 30 / 255) as u8,
        (color.b() as u16 * 30 / 255) as u8,
        60,
    );
    egui::Frame {
        inner_margin: egui::Margin::symmetric(8, 2),
        outer_margin: egui::Margin::ZERO,
        corner_radius: CornerRadius::same(20),
        shadow: Shadow::NONE,
        fill,
        stroke: Stroke::new(1.0, Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), 80)),
    }
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
}
