//! Reusable, theme-aware widgets — the building blocks every screen uses
//! so buttons, rows, and inputs look and behave the same everywhere
//! (sizes, hover animation, focus, truncation). Colors always come from
//! `theme::pal()`.

use egui::text::LayoutJob;
use egui::{
    Align2, Color32, CornerRadius, FontId, Galley, Id, Pos2, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, TextFormat, Ui, Vec2,
};
use std::sync::Arc;

use super::theme::{self, pal};

const HOVER_ANIM_SECS: f32 = 0.12;

// ─── Text helpers ────────────────────────────────────────────────────────────

/// Truncates `text` to at most `max_chars` characters (not bytes — safe
/// for emoji and non-Latin scripts), appending `…` when shortened.
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

/// Lays out `text` on a single line, eliding with `…` past `max_width`.
pub fn elided_galley(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

/// 0..=1 hover animation progress for a widget with `id`.
fn hover_t(ui: &Ui, id: Id, hovered: bool) -> f32 {
    ui.ctx()
        .animate_bool_with_time(id.with("hover"), hovered, HOVER_ANIM_SECS)
}

// ─── Buttons ─────────────────────────────────────────────────────────────────

/// A square, frameless icon button with an animated hover background and a
/// tooltip. `active` renders it as a toggled-on state.
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str, active: bool) -> Response {
    icon_button_sized(
        ui,
        icon,
        tooltip,
        active,
        theme::CONTROL_HEIGHT,
        theme::ICON_SIZE,
    )
}

pub fn icon_button_sized(
    ui: &mut Ui,
    icon: &str,
    tooltip: &str,
    active: bool,
    size: f32,
    icon_size: f32,
) -> Response {
    let p = pal();
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let enabled = ui.is_enabled();
    let hovered = response.hovered() && enabled;
    let fill = if active {
        p.accent_soft
    } else {
        p.hover.gamma_multiply(hover_t(ui, response.id, hovered))
    };
    if fill.a() > 0 {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_MD), fill);
    }
    let color = if !enabled {
        p.text_faint
    } else if active {
        p.accent
    } else if hovered {
        p.text
    } else {
        p.text_dim
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(icon_size),
        color,
    );
    if response.has_focus() {
        focus_ring(ui, rect);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tooltip)
    });
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// A text button with an optional leading icon, in one of the standard
/// kinds. Height is always `CONTROL_HEIGHT` so rows of buttons align.
pub fn button(ui: &mut Ui, kind: ButtonKind, icon: Option<&str>, label: &str) -> Response {
    let p = pal();
    let font = FontId::proportional(theme::TEXT_SM + 0.5);
    let enabled = ui.is_enabled();
    let (text_color, base, hover, stroke) = match kind {
        ButtonKind::Primary => (p.on_accent, p.accent, p.accent_hover, Stroke::NONE),
        ButtonKind::Secondary => (p.text, p.card, p.hover, Stroke::new(1.0, p.border)),
        ButtonKind::Ghost => (p.text_dim, Color32::TRANSPARENT, p.hover, Stroke::NONE),
        ButtonKind::Danger => (
            p.on_accent,
            p.danger,
            theme::blend(p.danger, Color32::BLACK, 0.12),
            Stroke::NONE,
        ),
    };
    let text_color = if enabled { text_color } else { p.text_faint };

    let label_galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, text_color);
    let icon_w = if icon.is_some() {
        theme::ICON_SIZE + 6.0
    } else {
        0.0
    };
    let padding_x = 12.0;
    let width = label_galley.size().x + icon_w + padding_x * 2.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme::CONTROL_HEIGHT), Sense::click());

    let t = hover_t(ui, response.id, response.hovered() && enabled);
    let fill = if !enabled {
        p.hover
    } else if base == Color32::TRANSPARENT {
        hover.gamma_multiply(t)
    } else {
        theme::blend(base, hover, t)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(theme::RADIUS_MD),
        fill,
        stroke,
        StrokeKind::Inside,
    );

    let mut x = rect.min.x + padding_x;
    if let Some(icon) = icon {
        ui.painter().text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            icon,
            FontId::proportional(theme::ICON_SIZE - 1.0),
            text_color,
        );
        x += icon_w;
    }
    ui.painter().galley(
        Pos2::new(x, rect.center().y - label_galley.size().y / 2.0),
        label_galley,
        text_color,
    );
    if response.has_focus() {
        focus_ring(ui, rect);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn primary_button(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    button(ui, ButtonKind::Primary, icon, label)
}

pub fn secondary_button(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    button(ui, ButtonKind::Secondary, icon, label)
}

pub fn ghost_button(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    button(ui, ButtonKind::Ghost, icon, label)
}

fn focus_ring(ui: &Ui, rect: Rect) {
    ui.painter().rect_stroke(
        rect.expand(2.0),
        CornerRadius::same(theme::RADIUS_MD + 2),
        Stroke::new(2.0, pal().accent.gamma_multiply(0.6)),
        StrokeKind::Outside,
    );
}

/// A menu entry with a leading icon and optional shortcut hint, for use
/// inside `menu_button` / `context_menu` popups.
pub fn menu_item(ui: &mut Ui, icon: &str, label: &str, shortcut: Option<&str>) -> Response {
    menu_item_colored(ui, icon, label, shortcut, pal().text)
}

pub fn menu_item_colored(
    ui: &mut Ui,
    icon: &str,
    label: &str,
    shortcut: Option<&str>,
    color: Color32,
) -> Response {
    let p = pal();
    let width = ui.available_width().max(200.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), p.hover);
    }
    ui.painter().text(
        Pos2::new(rect.min.x + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        icon,
        FontId::proportional(16.0),
        if color == p.text { p.text_dim } else { color },
    );
    ui.painter().text(
        Pos2::new(rect.min.x + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(theme::TEXT_SM + 0.5),
        color,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if let Some(shortcut) = shortcut {
        ui.painter().text(
            Pos2::new(rect.max.x - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            shortcut,
            FontId::proportional(theme::TEXT_XS),
            p.text_faint,
        );
    }
    response
}

// ─── Rows ────────────────────────────────────────────────────────────────────

pub struct RowSpec<'a> {
    pub icon: &'a str,
    pub icon_color: Color32,
    pub label: &'a str,
    /// Right-aligned secondary text (e.g. a count).
    pub trailing: Option<&'a str>,
    pub selected: bool,
    pub indent: f32,
    /// Width reserved on the right for hover actions drawn by the caller;
    /// the trailing text is hidden while hovered so they don't overlap.
    pub reserve_right: f32,
}

/// A full-width navigation/list row (file tree, filters, tags): icon,
/// single-line elided label, optional trailing count, hover & selected
/// states. Sense is click + drag so rows can act as drag sources.
pub fn list_row(ui: &mut Ui, spec: RowSpec<'_>) -> Response {
    let p = pal();
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, theme::CONTROL_HEIGHT),
        Sense::click_and_drag(),
    );
    let hovered = response.hovered() || response.context_menu_opened();

    let fill = if spec.selected {
        p.accent_soft
    } else {
        p.hover.gamma_multiply(hover_t(ui, response.id, hovered))
    };
    if fill.a() > 0 {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_MD), fill);
    }

    let mut x = rect.min.x + 8.0 + spec.indent;
    if !spec.icon.is_empty() {
        ui.painter().text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            spec.icon,
            FontId::proportional(theme::ICON_SIZE - 1.0),
            spec.icon_color,
        );
        x += 24.0;
    }

    let mut right = rect.max.x - 8.0;
    let show_actions = hovered && spec.reserve_right > 0.0;
    if let Some(trailing) = spec.trailing
        && !show_actions
    {
        let g = ui.painter().layout_no_wrap(
            trailing.to_owned(),
            FontId::proportional(theme::TEXT_XS),
            p.text_faint,
        );
        let pos = Pos2::new(right - g.size().x, rect.center().y - g.size().y / 2.0);
        right -= g.size().x + 8.0;
        ui.painter().galley(pos, g, p.text_faint);
    }
    if show_actions {
        right = rect.max.x - spec.reserve_right;
    }

    let font = if spec.selected {
        theme::semibold(theme::TEXT_SM + 0.5)
    } else {
        FontId::proportional(theme::TEXT_SM + 0.5)
    };
    let galley = elided_galley(ui, spec.label, font, p.text, right - x);
    ui.painter().galley(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley,
        p.text,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, spec.selected, spec.label)
    });
    response
}

/// A small icon button painted inside a row at `center` (hover actions).
pub fn row_action(ui: &mut Ui, id: Id, center: Pos2, icon: &str, tooltip: &str) -> Response {
    let p = pal();
    let rect = Rect::from_center_size(center, Vec2::splat(24.0));
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tooltip));
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), p.border);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(15.0),
        if resp.hovered() { p.text } else { p.text_dim },
    );
    resp.on_hover_text(tooltip)
}

/// Small uppercase section label.
pub fn section_header(ui: &mut Ui, text: &str) {
    ui.add_space(theme::SPACE_M);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(
            RichText::new(text.to_uppercase())
                .font(theme::semibold(11.0))
                .color(pal().text_faint),
        );
    });
    ui.add_space(2.0);
}

// ─── Inputs ──────────────────────────────────────────────────────────────────

/// Rounded search input with a leading magnifier, a clear button, and an
/// optional shortcut keycap shown while unfocused and empty. Returns the
/// text-edit response (`.changed()` also fires when cleared).
pub fn search_field(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    hint: &str,
    width: f32,
    shortcut_hint: Option<&str>,
) -> Response {
    let p = pal();
    let height = 32.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(id));
    ui.painter().rect(
        rect,
        CornerRadius::same(theme::RADIUS_MD),
        if focused { p.card } else { p.surface },
        Stroke::new(1.0, if focused { p.accent } else { p.border }),
        StrokeKind::Inside,
    );
    ui.painter().text(
        Pos2::new(rect.min.x + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        egui_icons::icons::ICON_SEARCH.codepoint,
        FontId::proportional(16.0),
        p.text_faint,
    );

    let mut cleared = false;
    let mut right_pad = 10.0;
    if !text.is_empty() {
        let clear_rect = Rect::from_center_size(
            Pos2::new(rect.max.x - 16.0, rect.center().y),
            Vec2::splat(22.0),
        );
        let clear = ui.interact(clear_rect, id.with("clear"), Sense::click());
        if clear.hovered() {
            ui.painter()
                .rect_filled(clear_rect, CornerRadius::same(theme::RADIUS_SM), p.hover);
        }
        ui.painter().text(
            clear_rect.center(),
            Align2::CENTER_CENTER,
            egui_icons::icons::ICON_CLOSE.codepoint,
            FontId::proportional(14.0),
            p.text_dim,
        );
        cleared = clear.clicked();
        right_pad = 32.0;
    } else if let (Some(hint_text), false) = (shortcut_hint, focused) {
        let g = ui.painter().layout_no_wrap(
            hint_text.to_owned(),
            FontId::proportional(theme::TEXT_XS),
            p.text_faint,
        );
        let kbd = Rect::from_min_size(
            Pos2::new(rect.max.x - g.size().x - 18.0, rect.center().y - 10.0),
            Vec2::new(g.size().x + 10.0, 20.0),
        );
        ui.painter().rect(
            kbd,
            CornerRadius::same(theme::RADIUS_SM),
            p.card,
            Stroke::new(1.0, p.border),
            StrokeKind::Inside,
        );
        ui.painter().galley(
            Pos2::new(kbd.min.x + 5.0, kbd.center().y - g.size().y / 2.0),
            g,
            p.text_faint,
        );
        right_pad = kbd.width() + 16.0;
    }

    let edit_rect = Rect::from_min_max(
        Pos2::new(rect.min.x + 32.0, rect.min.y),
        Pos2::new(rect.max.x - right_pad, rect.max.y),
    );
    let mut response = ui.put(
        edit_rect,
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(RichText::new(hint).color(p.text_faint))
            .frame(egui::Frame::NONE)
            .vertical_align(egui::Align::Center)
            .font(FontId::proportional(theme::TEXT_BODY)),
    );
    if cleared {
        text.clear();
        response.mark_changed();
    }
    response
}

// ─── Composite blocks ────────────────────────────────────────────────────────

/// Segmented control of `(icon, label)` options. Returns the newly clicked
/// index, if any.
pub fn segmented(ui: &mut Ui, id: Id, options: &[(&str, &str)], selected: usize) -> Option<usize> {
    let p = pal();
    let mut clicked = None;
    let font = FontId::proportional(theme::TEXT_SM);
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, label)| {
            ui.painter()
                .layout_no_wrap((*label).to_owned(), font.clone(), p.text)
        })
        .collect();
    let seg_widths: Vec<f32> = galleys
        .iter()
        .zip(options)
        .map(|(g, (icon, _))| g.size().x + if icon.is_empty() { 20.0 } else { 40.0 })
        .collect();
    let total: f32 = seg_widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, 32.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS_MD), p.surface);

    let mut x = rect.min.x + 2.0;
    for (i, ((icon, _), galley)) in options.iter().zip(galleys).enumerate() {
        let seg = Rect::from_min_size(
            Pos2::new(x, rect.min.y + 2.0),
            Vec2::new(seg_widths[i], 28.0),
        );
        let resp = ui.interact(seg, id.with(i), Sense::click());
        let is_sel = i == selected;
        if is_sel {
            ui.painter().rect(
                seg,
                CornerRadius::same(theme::RADIUS_SM),
                p.card,
                Stroke::new(1.0, p.border),
                StrokeKind::Inside,
            );
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(seg, CornerRadius::same(theme::RADIUS_SM), p.hover);
        }
        let color = if is_sel { p.text } else { p.text_dim };
        let mut tx = seg.min.x + 10.0;
        if !icon.is_empty() {
            ui.painter().text(
                Pos2::new(tx, seg.center().y),
                Align2::LEFT_CENTER,
                *icon,
                FontId::proportional(15.0),
                if is_sel { p.accent } else { color },
            );
            tx += 20.0;
        }
        ui.painter().galley(
            Pos2::new(tx, seg.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            && !is_sel
        {
            clicked = Some(i);
        }
        x += seg_widths[i];
    }
    clicked
}

/// Centered empty-state block: big muted icon, title, explanation, and an
/// optional primary `(icon, label)` action. Returns true when clicked.
pub fn empty_state(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    body: &str,
    action: Option<(&str, &str)>,
) -> bool {
    let p = pal();
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(theme::SPACE_XL * 2.5);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 32.0, p.hover);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon,
            FontId::proportional(30.0),
            p.text_dim,
        );
        ui.add_space(theme::SPACE_L);
        ui.label(
            RichText::new(title)
                .font(theme::semibold(theme::TEXT_LG))
                .color(p.text),
        );
        ui.add_space(theme::SPACE_XS);
        ui.scope(|ui| {
            ui.set_max_width(380.0);
            ui.label(RichText::new(body).size(theme::TEXT_BODY).color(p.text_dim));
        });
        if let Some((action_icon, label)) = action {
            ui.add_space(theme::SPACE_L);
            let icon = if action_icon.is_empty() {
                None
            } else {
                Some(action_icon)
            };
            clicked = primary_button(ui, icon, label).clicked();
        }
    });
    clicked
}

/// Dims everything behind a modal (fades in). Returns true when the
/// backdrop itself is clicked, so callers can close on outside click.
pub fn modal_backdrop(ctx: &egui::Context, id: Id) -> bool {
    let screen = ctx.viewport_rect();
    let t = ctx.animate_bool_with_time(id.with("fade"), true, 0.15);
    let alpha = if pal().is_dark { 150.0 } else { 90.0 };
    let mut clicked = false;
    egui::Area::new(id)
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let resp = ui.allocate_rect(screen, Sense::click());
            ui.painter().rect_filled(
                screen,
                CornerRadius::ZERO,
                Color32::from_black_alpha((alpha * t) as u8),
            );
            clicked = resp.clicked();
        });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_is_utf8_safe() {
        assert_eq!(truncate_chars("pendek", 10), "pendek");
        assert_eq!(truncate_chars("abcdefghij", 5), "abcd…");
        // Multi-byte characters must never be split mid-codepoint.
        assert_eq!(truncate_chars("🎨🎨🎨🎨🎨", 3), "🎨🎨…");
        assert_eq!(truncate_chars("日本語のメモです", 4), "日本語…");
        assert_eq!(truncate_chars("", 3), "");
    }
}
