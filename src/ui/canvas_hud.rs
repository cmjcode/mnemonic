//! In-canvas HUD pills: zoom controls (bottom-right) and the stroke color /
//! width picker (bottom-center).

use egui::{CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use egui_icons::icons::{ICON_ADD, ICON_FIT_SCREEN, ICON_REMOVE};

use crate::i18n::LocaleManager;
use crate::ui::theme::{self, pal};
use crate::ui::widgets;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CanvasHudEvent {
    ZoomIn,
    ZoomOut,
    ResetZoom,
    SetStrokeColor([f32; 3]),
    SetStrokeWidth(f32),
}

/// `(rgb, locale key)` color presets for strokes and sticky notes.
const COLOR_PRESETS: &[([f32; 3], &str)] = &[
    ([1.0, 0.94, 0.55], "color-yellow"),
    ([0.65, 0.85, 1.0], "color-blue"),
    ([0.68, 0.94, 0.72], "color-green"),
    ([1.0, 0.75, 0.85], "color-pink"),
    ([0.85, 0.75, 1.0], "color-purple"),
    ([1.0, 0.6, 0.1], "color-orange"),
    ([1.0, 0.3, 0.35], "color-red"),
    ([0.25, 0.28, 0.35], "color-graphite"),
];

pub struct CanvasHud;

impl CanvasHud {
    pub fn show_zoom_hud(
        ui: &mut Ui,
        tr: &LocaleManager,
        zoom_level: f32,
    ) -> Option<CanvasHudEvent> {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let mut event = None;
        theme::pill_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if widgets::icon_button_sized(
                    ui,
                    ICON_REMOVE.codepoint,
                    &t("canvas-zoom-out"),
                    false,
                    28.0,
                    16.0,
                )
                .clicked()
                {
                    event = Some(CanvasHudEvent::ZoomOut);
                }
                let pct = RichText::new(format!("{}%", (zoom_level * 100.0).round() as i32))
                    .size(theme::TEXT_SM)
                    .color(p.text_dim);
                if ui
                    .add_sized(
                        Vec2::new(48.0, 28.0),
                        egui::Label::new(pct).sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(t("canvas-zoom-reset"))
                    .clicked()
                {
                    event = Some(CanvasHudEvent::ResetZoom);
                }
                if widgets::icon_button_sized(
                    ui,
                    ICON_ADD.codepoint,
                    &t("canvas-zoom-in"),
                    false,
                    28.0,
                    16.0,
                )
                .clicked()
                {
                    event = Some(CanvasHudEvent::ZoomIn);
                }
                if widgets::icon_button_sized(
                    ui,
                    ICON_FIT_SCREEN.codepoint,
                    &t("canvas-zoom-reset"),
                    false,
                    28.0,
                    16.0,
                )
                .clicked()
                {
                    event = Some(CanvasHudEvent::ResetZoom);
                }
            });
        });
        event
    }

    pub fn show_style_hud(
        ui: &mut Ui,
        tr: &LocaleManager,
        current_color: [f32; 3],
        current_width: f32,
    ) -> Option<CanvasHudEvent> {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let mut event = None;

        theme::pill_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (col, key) in COLOR_PRESETS {
                    let active = col
                        .iter()
                        .zip(current_color.iter())
                        .all(|(a, b)| (a - b).abs() < 0.05);
                    let c32 = egui::Color32::from_rgb(
                        (col[0] * 255.0) as u8,
                        (col[1] * 255.0) as u8,
                        (col[2] * 255.0) as u8,
                    );
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                    if active || resp.hovered() {
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(theme::RADIUS_MD),
                            if active { p.accent_soft } else { p.hover },
                            if active {
                                Stroke::new(1.0, p.accent)
                            } else {
                                Stroke::NONE
                            },
                            StrokeKind::Inside,
                        );
                    }
                    ui.painter()
                        .circle(rect.center(), 8.0, c32, Stroke::new(1.0, p.border_strong));
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(t(key))
                        .clicked()
                    {
                        event = Some(CanvasHudEvent::SetStrokeColor(*col));
                    }
                }

                ui.add(egui::Separator::default().spacing(8.0));

                for (w, key) in [
                    (2.0, "canvas-width-thin"),
                    (4.0, "canvas-width-medium"),
                    (7.0, "canvas-width-thick"),
                ] {
                    let active = (current_width - w).abs() < 0.5;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                    if active || resp.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(theme::RADIUS_MD),
                            if active { p.accent_soft } else { p.hover },
                        );
                    }
                    ui.painter().line_segment(
                        [
                            rect.left_center() + Vec2::new(8.0, 0.0),
                            rect.right_center() - Vec2::new(8.0, 0.0),
                        ],
                        Stroke::new(w * 0.8, if active { p.accent } else { p.text_dim }),
                    );
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(t(key))
                        .clicked()
                    {
                        event = Some(CanvasHudEvent::SetStrokeWidth(w));
                    }
                }
            });
        });

        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canvas_hud_event_variants() {
        let ev = CanvasHudEvent::SetStrokeWidth(4.0);
        assert_eq!(ev, CanvasHudEvent::SetStrokeWidth(4.0));
        assert_eq!(COLOR_PRESETS.len(), 8);
    }
}
