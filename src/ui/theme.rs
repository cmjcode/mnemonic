//! Design system: color palettes (dark & light), type scale, spacing,
//! reusable frames, and the egui `Style` built from them.
//!
//! Components never hard-code colors; they read the active palette via
//! [`pal()`], which [`apply_theme`] switches. That single indirection is
//! what makes Light mode actually work — every widget follows the theme
//! instead of carrying dark-only constants.

use std::sync::atomic::{AtomicBool, Ordering};

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin, Shadow,
    Stroke, Style, Vec2, Visuals,
};

// ─── Spacing, Radius & Type Scale ────────────────────────────────────────────

pub const SPACE_XS: f32 = 4.0;
pub const SPACE_S: f32 = 8.0;
pub const SPACE_M: f32 = 12.0;
pub const SPACE_L: f32 = 16.0;
pub const SPACE_XL: f32 = 24.0;

pub const RADIUS_SM: u8 = 6;
pub const RADIUS_MD: u8 = 8;
pub const RADIUS_LG: u8 = 12;

pub const TEXT_XS: f32 = 12.0;
pub const TEXT_SM: f32 = 13.0;
pub const TEXT_BODY: f32 = 14.0;
pub const TEXT_LG: f32 = 17.0;
pub const TEXT_XL: f32 = 22.0;
pub const TEXT_DISPLAY: f32 = 28.0;

/// Minimum clickable size for any icon button or row.
pub const CONTROL_HEIGHT: f32 = 30.0;
pub const ICON_SIZE: f32 = 17.0;

pub const TOPBAR_HEIGHT: f32 = 48.0;
pub const SIDEBAR_WIDTH: f32 = 264.0;
pub const CHAT_SIDEBAR_WIDTH: f32 = 380.0;
/// Comfortable reading/writing column width for the note editor.
pub const EDITOR_MAX_WIDTH: f32 = 760.0;
pub const GRID_GAP: f32 = 12.0;

/// Name of the semibold font family registered by [`apply_theme`].
pub const SEMIBOLD_FAMILY: &str = "semibold";

// ─── Palette ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub is_dark: bool,
    /// Main content background.
    pub bg: Color32,
    /// Side panels & top bar.
    pub surface: Color32,
    /// Cards, inputs, popovers.
    pub card: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Accent-tinted background for selected rows / active toggles.
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub success: Color32,
    pub warning: Color32,
    /// Type colors used for file icons.
    pub note_icon: Color32,
    pub canvas_icon: Color32,
    pub pdf_icon: Color32,
    /// CSV/XLSX sheets (§3.8): spreadsheet green.
    pub sheet_icon: Color32,
    pub folder_icon: Color32,
    pub shadow: Color32,
}

pub const DARK: Palette = Palette {
    is_dark: true,
    bg: Color32::from_rgb(22, 23, 26),
    surface: Color32::from_rgb(28, 29, 33),
    card: Color32::from_rgb(35, 36, 41),
    hover: Color32::from_rgb(43, 45, 51),
    border: Color32::from_rgb(46, 48, 54),
    border_strong: Color32::from_rgb(62, 65, 73),
    text: Color32::from_rgb(236, 236, 238),
    text_dim: Color32::from_rgb(163, 166, 174),
    text_faint: Color32::from_rgb(112, 115, 124),
    accent: Color32::from_rgb(74, 128, 240),
    accent_hover: Color32::from_rgb(98, 148, 250),
    accent_soft: Color32::from_rgb(38, 52, 82),
    on_accent: Color32::WHITE,
    danger: Color32::from_rgb(240, 97, 109),
    danger_soft: Color32::from_rgb(70, 36, 42),
    success: Color32::from_rgb(63, 185, 123),
    warning: Color32::from_rgb(232, 169, 58),
    note_icon: Color32::from_rgb(120, 165, 255),
    canvas_icon: Color32::from_rgb(232, 169, 58),
    pdf_icon: Color32::from_rgb(240, 97, 109),
    sheet_icon: Color32::from_rgb(63, 185, 123),
    folder_icon: Color32::from_rgb(140, 145, 158),
    shadow: Color32::from_black_alpha(90),
};

pub const LIGHT: Palette = Palette {
    is_dark: false,
    bg: Color32::from_rgb(248, 248, 250),
    surface: Color32::from_rgb(242, 242, 245),
    card: Color32::WHITE,
    hover: Color32::from_rgb(232, 233, 238),
    border: Color32::from_rgb(224, 225, 230),
    border_strong: Color32::from_rgb(204, 206, 213),
    text: Color32::from_rgb(29, 30, 34),
    text_dim: Color32::from_rgb(88, 91, 99),
    text_faint: Color32::from_rgb(137, 140, 149),
    accent: Color32::from_rgb(47, 111, 235),
    accent_hover: Color32::from_rgb(33, 94, 212),
    accent_soft: Color32::from_rgb(222, 232, 252),
    on_accent: Color32::WHITE,
    danger: Color32::from_rgb(214, 58, 74),
    danger_soft: Color32::from_rgb(252, 228, 231),
    success: Color32::from_rgb(30, 150, 90),
    warning: Color32::from_rgb(185, 122, 16),
    note_icon: Color32::from_rgb(47, 111, 235),
    canvas_icon: Color32::from_rgb(200, 128, 10),
    pdf_icon: Color32::from_rgb(214, 58, 74),
    sheet_icon: Color32::from_rgb(30, 140, 84),
    folder_icon: Color32::from_rgb(120, 124, 134),
    shadow: Color32::from_black_alpha(28),
};

static DARK_ACTIVE: AtomicBool = AtomicBool::new(true);

/// The palette of the theme most recently applied with [`apply_theme`].
pub fn pal() -> &'static Palette {
    if DARK_ACTIVE.load(Ordering::Relaxed) {
        &DARK
    } else {
        &LIGHT
    }
}

/// Linear blend between two colors (`t = 0` → `a`, `t = 1` → `b`).
pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

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

    /// Stable id used in the settings file.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn from_str_or_default(s: &str) -> ThemeMode {
        match s {
            "light" => ThemeMode::Light,
            _ => ThemeMode::Dark,
        }
    }

    pub fn palette(self) -> &'static Palette {
        match self {
            ThemeMode::Light => &LIGHT,
            ThemeMode::Dark => &DARK,
        }
    }

    fn visuals(self) -> Visuals {
        let p = self.palette();
        let mut v = if p.is_dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        let radius = CornerRadius::same(RADIUS_MD);

        v.panel_fill = p.surface;
        v.window_fill = p.card;
        v.faint_bg_color = p.surface;
        v.extreme_bg_color = p.card;
        v.code_bg_color = p.hover;
        v.window_stroke = Stroke::new(1.0, p.border);
        v.window_corner_radius = CornerRadius::same(RADIUS_LG);
        v.menu_corner_radius = radius;
        v.window_shadow = Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: p.shadow,
        };
        v.popup_shadow = Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: p.shadow,
        };
        v.hyperlink_color = p.accent;
        v.warn_fg_color = p.warning;
        v.error_fg_color = p.danger;
        v.text_cursor.stroke = Stroke::new(2.0, p.accent);
        v.selection.bg_fill = p.accent.gamma_multiply(if p.is_dark { 0.35 } else { 0.25 });
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.override_text_color = None;

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.surface;
        w.noninteractive.weak_bg_fill = p.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        w.noninteractive.corner_radius = radius;

        w.inactive.bg_fill = p.card;
        w.inactive.weak_bg_fill = p.card;
        w.inactive.bg_stroke = Stroke::new(1.0, p.border);
        w.inactive.fg_stroke = Stroke::new(1.0, p.text);
        w.inactive.corner_radius = radius;

        w.hovered.bg_fill = p.hover;
        w.hovered.weak_bg_fill = p.hover;
        w.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
        w.hovered.fg_stroke = Stroke::new(1.0, p.text);
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = p.accent_soft;
        w.active.weak_bg_fill = p.accent_soft;
        w.active.bg_stroke = Stroke::new(1.0, p.accent);
        w.active.fg_stroke = Stroke::new(1.0, p.text);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;

        w.open.bg_fill = p.hover;
        w.open.weak_bg_fill = p.hover;
        w.open.bg_stroke = Stroke::new(1.0, p.border_strong);
        w.open.fg_stroke = Stroke::new(1.0, p.text);
        w.open.corner_radius = radius;
        v
    }
}

// ─── Fonts ───────────────────────────────────────────────────────────────────

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");

/// Registers Inter as the proportional UI font (egui's defaults stay as
/// fallbacks for emoji & symbols) plus a `semibold` family for headings,
/// then the Material icon fonts. Idempotent per context.
///
/// Font changes only take effect on the *next* frame, so this must run
/// before the first frame (`main.rs` calls it from the creation context);
/// otherwise the first layout using [`semibold`] would find no font.
pub fn install_fonts(ctx: &egui::Context) {
    let installed_id = egui::Id::new("mnemonic_fonts_installed");
    if ctx
        .data(|d| d.get_temp::<bool>(installed_id))
        .unwrap_or(false)
    {
        return;
    }

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".to_owned(),
        std::sync::Arc::new(FontData::from_static(INTER_REGULAR)),
    );
    fonts.font_data.insert(
        "Inter-SemiBold".to_owned(),
        std::sync::Arc::new(FontData::from_static(INTER_SEMIBOLD)),
    );

    let proportional_fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".to_owned());

    let mut semibold = vec!["Inter-SemiBold".to_owned(), "Inter".to_owned()];
    semibold.extend(proportional_fallbacks);
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD_FAMILY.into()), semibold);

    ctx.set_fonts(fonts);
    egui_icons::initialize(ctx);
    ctx.data_mut(|d| d.insert_temp(installed_id, true));
}

/// A semibold `FontId` — egui has no bold weight of its own.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD_FAMILY.into()))
}

// ─── Apply Theme to Context ──────────────────────────────────────────────────

/// Applies `mode`'s palette and the app-wide spacing/type scale. Cheap to
/// call, but the app only calls it when the mode actually changes.
pub fn apply_theme(ctx: &egui::Context, mode: ThemeMode) {
    install_fonts(ctx);
    DARK_ACTIVE.store(mode == ThemeMode::Dark, Ordering::Relaxed);

    let theme = match mode {
        ThemeMode::Dark => egui::Theme::Dark,
        ThemeMode::Light => egui::Theme::Light,
    };
    ctx.set_theme(theme);

    let mut style = Style {
        visuals: mode.visuals(),
        ..Default::default()
    };
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.item_spacing = Vec2::new(SPACE_S, 6.0);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.window_margin = Margin::same(16);
    style.spacing.icon_width = 16.0;
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    style.spacing.scroll.bar_width = 8.0;

    use egui::TextStyle::*;
    style
        .text_styles
        .insert(Small, FontId::proportional(TEXT_XS));
    style
        .text_styles
        .insert(Body, FontId::proportional(TEXT_BODY));
    style
        .text_styles
        .insert(Button, FontId::proportional(TEXT_SM + 0.5));
    style
        .text_styles
        .insert(Monospace, FontId::monospace(TEXT_SM + 0.5));
    style.text_styles.insert(Heading, semibold(TEXT_LG + 1.0));

    ctx.set_style_of(theme, style);
}

// ─── Frames ──────────────────────────────────────────────────────────────────

/// Content card (note/PDF cards, grouped settings).
pub fn card_frame() -> Frame {
    let p = pal();
    Frame {
        inner_margin: Margin::same(14),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::same(RADIUS_LG),
        shadow: Shadow {
            offset: [0, 1],
            blur: 3,
            spread: 0,
            color: p.shadow.gamma_multiply(0.5),
        },
        fill: p.card,
        stroke: Stroke::new(1.0, p.border),
    }
}

/// Floating surfaces: modals, the command palette, canvas tool docks.
pub fn popover_frame() -> Frame {
    let p = pal();
    Frame {
        inner_margin: Margin::same(12),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::same(RADIUS_LG),
        shadow: Shadow {
            offset: [0, 10],
            blur: 32,
            spread: 0,
            color: p.shadow,
        },
        fill: p.card,
        stroke: Stroke::new(1.0, p.border),
    }
}

/// Compact capsule (zoom pill, style picker).
pub fn pill_frame() -> Frame {
    popover_frame()
        .inner_margin(Margin::symmetric(8, 4))
        .corner_radius(CornerRadius::same(RADIUS_LG))
}

/// Docked side panel (file tree sidebar, AI sidebar, outline).
pub fn side_panel_frame() -> Frame {
    let p = pal();
    Frame {
        inner_margin: Margin::symmetric(10, 10),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::ZERO,
        shadow: Shadow::NONE,
        fill: p.surface,
        stroke: Stroke::NONE,
    }
}

/// The top application bar.
pub fn top_bar_frame() -> Frame {
    let p = pal();
    Frame {
        inner_margin: Margin::symmetric(10, 0),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::ZERO,
        shadow: Shadow::NONE,
        fill: p.bg,
        stroke: Stroke::NONE,
    }
}

/// Main content area.
pub fn content_frame() -> Frame {
    Frame {
        inner_margin: Margin::ZERO,
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::ZERO,
        shadow: Shadow::NONE,
        fill: pal().bg,
        stroke: Stroke::NONE,
    }
}

/// Compact pill-style frame for tag chips.
pub fn tag_chip_frame(color: Color32) -> Frame {
    let p = pal();
    Frame {
        inner_margin: Margin::symmetric(8, 2),
        outer_margin: Margin::ZERO,
        corner_radius: CornerRadius::same(RADIUS_LG),
        shadow: Shadow::NONE,
        fill: blend(p.card, color, if p.is_dark { 0.16 } else { 0.12 }),
        stroke: Stroke::NONE,
    }
}

// ─── Card Colour Palette ──────────────────────────────────────────────────────

/// Note colors the user can pick, as `(settings id, solid color)`.
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

pub fn color_solid_for(name: Option<&str>) -> Option<Color32> {
    let name = name?;
    PALETTE_SOLID
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
}

/// Card background for a note colored `name`: the card color gently
/// tinted toward the solid color, readable in both themes.
pub fn note_tint(name: Option<&str>) -> Option<Color32> {
    let p = pal();
    color_solid_for(name).map(|c| blend(p.card, c, if p.is_dark { 0.18 } else { 0.14 }))
}

// ─── Tag Colours ──────────────────────────────────────────────────────────────

const TAG_COLORS: &[Color32] = &[
    Color32::from_rgb(92, 145, 255),  // blue
    Color32::from_rgb(63, 185, 123),  // green
    Color32::from_rgb(175, 110, 230), // purple
    Color32::from_rgb(232, 150, 50),  // orange
    Color32::from_rgb(235, 90, 90),   // red
    Color32::from_rgb(40, 180, 200),  // teal
    Color32::from_rgb(230, 80, 150),  // pink
    Color32::from_rgb(200, 165, 20),  // yellow
];

pub fn tag_color(tag: &str) -> Color32 {
    let mut hash: u32 = 2166136261;
    for b in tag.to_lowercase().as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    TAG_COLORS[(hash as usize) % TAG_COLORS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_tint_known_and_unknown_names() {
        assert!(note_tint(Some("yellow")).is_some());
        assert_eq!(note_tint(Some("not-a-color")), None);
        assert_eq!(note_tint(None), None);
    }

    #[test]
    fn tag_color_is_deterministic() {
        assert_eq!(tag_color("rumah"), tag_color("rumah"));
        assert_eq!(tag_color("Rumah"), tag_color("rumah")); // case-insensitive
    }

    #[test]
    fn apply_theme_switches_active_palette() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, ThemeMode::Light);
        assert!(!pal().is_dark);
        apply_theme(&ctx, ThemeMode::Dark);
        assert!(pal().is_dark);
        assert_eq!(ThemeMode::Dark.toggled(), ThemeMode::Light);
    }

    #[test]
    fn theme_mode_round_trips_through_settings_id() {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            assert_eq!(ThemeMode::from_str_or_default(mode.as_str()), mode);
        }
        assert_eq!(ThemeMode::from_str_or_default("garbage"), ThemeMode::Dark);
    }

    /// Body text must stay readable (WCAG AA, 4.5:1) on every surface in
    /// both themes; dim text at least 3:1.
    #[test]
    fn text_contrast_meets_minimums() {
        fn luminance(c: Color32) -> f32 {
            let ch = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        }
        fn contrast(a: Color32, b: Color32) -> f32 {
            let (la, lb) = (luminance(a), luminance(b));
            (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
        }
        for p in [&DARK, &LIGHT] {
            for bg in [p.bg, p.surface, p.card, p.hover] {
                assert!(contrast(p.text, bg) >= 4.5, "text on {bg:?}");
                assert!(contrast(p.text_dim, bg) >= 3.0, "text_dim on {bg:?}");
            }
            assert!(contrast(p.on_accent, p.accent) >= 3.0, "on_accent");
        }
    }
}
