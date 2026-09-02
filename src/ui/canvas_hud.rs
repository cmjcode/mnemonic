//! In-Canvas HUD Pills bergaya Shapr3D / DUCAD.
//!
//! Menampilkan kontrol mengambang ringkas di pojok kanvas/editor untuk
//! zoom, pemilihan warna goresan, ketebalan garis, dan status kanvas.

use egui::{
    Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use egui_icons::icons::{
    ICON_ADD, ICON_FIT_SCREEN, ICON_PALETTE, ICON_REMOVE, ICON_RESTART_ALT, ICON_ZOOM_IN,
    ICON_ZOOM_OUT,
};

use crate::ui::theme::{
    pill_frame, ACCENT_BLUE, BG_CARD_DARK, BG_HOVER_DARK, BORDER_SUBTLE, ROUNDING_LG,
    ROUNDING_MD, ROUNDING_SM, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CanvasHudEvent {
    ZoomIn,
    ZoomOut,
    ResetZoom,
    FitContent,
    SetStrokeColor([f32; 3]),
    SetStrokeWidth(f32),
}

pub struct CanvasHud;

impl CanvasHud {
    /// Render zoom HUD pill di pojok kanan bawah kanvas.
    pub fn show_zoom_hud(ui: &mut Ui, zoom_level: f32) -> Option<CanvasHudEvent> {
        let mut event = None;

        pill_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                // Zoom Out (-)
                let (minus_rect, minus_resp) =
                    ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                if minus_resp.hovered() {
                    ui.painter().rect(
                        minus_rect,
                        CornerRadius::same(ROUNDING_SM),
                        BG_HOVER_DARK,
                        Stroke::NONE,
                        StrokeKind::Inside,
                    );
                }
                ui.painter().text(
                    minus_rect.center(),
                    Align2::CENTER_CENTER,
                    ICON_REMOVE.codepoint,
                    egui::FontId::proportional(13.0),
                    TEXT_PRIMARY,
                );
                if minus_resp.on_hover_text("Perkecil Tampilan").clicked() {
                    event = Some(CanvasHudEvent::ZoomOut);
                }

                // Zoom percentage label & reset on click
                let zoom_pct = (zoom_level * 100.0).round() as i32;
                let pct_text = RichText::new(format!("{zoom_pct}%"))
                    .size(11.5)
                    .color(TEXT_SECONDARY);

                let label_resp = ui.add(egui::Label::new(pct_text).sense(Sense::click()));
                if label_resp
                    .on_hover_text("Klik untuk reset ke 100%")
                    .clicked()
                {
                    event = Some(CanvasHudEvent::ResetZoom);
                }

                // Zoom In (+)
                let (plus_rect, plus_resp) =
                    ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                if plus_resp.hovered() {
                    ui.painter().rect(
                        plus_rect,
                        CornerRadius::same(ROUNDING_SM),
                        BG_HOVER_DARK,
                        Stroke::NONE,
                        StrokeKind::Inside,
                    );
                }
                ui.painter().text(
                    plus_rect.center(),
                    Align2::CENTER_CENTER,
                    ICON_ADD.codepoint,
                    egui::FontId::proportional(13.0),
                    TEXT_PRIMARY,
                );
                if plus_resp.on_hover_text("Perbesar Tampilan").clicked() {
                    event = Some(CanvasHudEvent::ZoomIn);
                }
            });
        });

        event
    }

    /// Render style picker HUD pill untuk warna stroke & ketebalan.
    pub fn show_style_hud(
        ui: &mut Ui,
        current_color: [f32; 3],
        current_width: f32,
    ) -> Option<CanvasHudEvent> {
        let mut event = None;

        pill_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                let color_presets: &[([f32; 3], &str)] = &[
                    ([0.9, 0.9, 0.95], "Putih"),
                    ([0.2, 0.6, 1.0], "Biru"),
                    ([0.2, 0.85, 0.4], "Hijau"),
                    ([1.0, 0.6, 0.1], "Oranye"),
                    ([1.0, 0.3, 0.35], "Merah"),
                    ([0.7, 0.4, 0.95], "Ungu"),
                    ([1.0, 0.85, 0.2], "Kuning"),
                ];

                for (col, label) in color_presets {
                    let is_active = (col[0] - current_color[0]).abs() < 0.05
                        && (col[1] - current_color[1]).abs() < 0.05
                        && (col[2] - current_color[2]).abs() < 0.05;

                    let c32 = Color32::from_rgb(
                        (col[0] * 255.0) as u8,
                        (col[1] * 255.0) as u8,
                        (col[2] * 255.0) as u8,
                    );

                    let (dot_rect, dot_resp) =
                        ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());

                    if is_active {
                        ui.painter().rect(
                            dot_rect,
                            CornerRadius::same(ROUNDING_SM),
                            Color32::from_rgba_premultiplied(10, 132, 255, 60),
                            Stroke::new(1.0, ACCENT_BLUE),
                            StrokeKind::Inside,
                        );
                    }

                    ui.painter().circle_filled(dot_rect.center(), 5.5, c32);

                    if dot_resp.on_hover_text(*label).clicked() {
                        event = Some(CanvasHudEvent::SetStrokeColor(*col));
                    }
                }

                ui.add_space(4.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(4.0);

                let widths = [(2.0, "Tipis"), (4.0, "Sedang"), (7.0, "Tebal")];
                for (w, w_label) in widths {
                    let is_w_active = (current_width - w).abs() < 0.5;
                    let (w_rect, w_resp) =
                        ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());

                    if is_w_active {
                        ui.painter().rect(
                            w_rect,
                            CornerRadius::same(ROUNDING_SM),
                            BG_CARD_DARK,
                            Stroke::new(1.0, ACCENT_BLUE),
                            StrokeKind::Inside,
                        );
                    } else if w_resp.hovered() {
                        ui.painter().rect(
                            w_rect,
                            CornerRadius::same(ROUNDING_SM),
                            BG_HOVER_DARK,
                            Stroke::NONE,
                            StrokeKind::Inside,
                        );
                    }

                    ui.painter().circle_filled(
                        w_rect.center(),
                        (w * 0.7).clamp(2.0, 6.0),
                        if is_w_active { ACCENT_BLUE } else { TEXT_SECONDARY },
                    );

                    if w_resp.on_hover_text(w_label).clicked() {
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
    }
}
