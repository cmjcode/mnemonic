//! Bilah Alat Vertikal Kiri (Left Floating Tool Dock) bergaya Shapr3D / DUCAD.
//!
//! Menampilkan kolom vertikal ramping mengambang di sisi kiri kanvas/PDF viewer,
//! dengan active tool states, tooltips, dan shortcut indicator.

use egui::{
    Align2, Color32, CornerRadius, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use egui_icons::icons::{
    ICON_ADS_CLICK, ICON_ARROWS_OUTWARD, ICON_BRUSH, ICON_CIRCLE, ICON_CROP_16_9, ICON_DELETE,
    ICON_PAN_TOOL, ICON_REDO, ICON_STICKY_NOTE_2, ICON_UNDO,
};

use crate::canvas::element::ShapeKind;
use crate::canvas::CanvasTool;
use crate::ui::theme::{
    glass_frame, ACCENT_BLUE, BG_HOVER_DARK, BORDER_SUBTLE, ICON_SIZE_DEFAULT, ROUNDING_SM,
    TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftToolbarEvent {
    SelectCanvasTool(CanvasTool),
    Undo,
    Redo,
    ZoomIn,
    ZoomOut,
    ResetZoom,
}

pub struct LeftToolbar;

impl LeftToolbar {
    /// Render floating tool dock untuk Canvas Whiteboard mode.
    pub fn show_canvas(
        ui: &mut Ui,
        active_tool: CanvasTool,
        can_undo: bool,
        can_redo: bool,
    ) -> Option<LeftToolbarEvent> {
        let mut event = None;
        let icon_sz = ICON_SIZE_DEFAULT;

        glass_frame().show(ui, |ui| {
            ui.set_width(36.0);
            ui.vertical_centered(|ui| {
                ui.add_space(2.0);

                let tools = [
                    (CanvasTool::Select, ICON_ADS_CLICK.codepoint, "Pilih / Ubah (V)"),
                    (CanvasTool::Pan, ICON_PAN_TOOL.codepoint, "Geser Kanvas (H / Space)"),
                    (CanvasTool::StickyNote, ICON_STICKY_NOTE_2.codepoint, "Catatan Tempel (S)"),
                    (
                        CanvasTool::Shape(ShapeKind::Rectangle),
                        ICON_CROP_16_9.codepoint,
                        "Persegi (R)",
                    ),
                    (
                        CanvasTool::Shape(ShapeKind::Ellipse),
                        ICON_CIRCLE.codepoint,
                        "Lingkaran / Elips (O)",
                    ),
                    (
                        CanvasTool::Connector,
                        ICON_ARROWS_OUTWARD.codepoint,
                        "Konektor / Panah (A)",
                    ),
                    (CanvasTool::Pen, ICON_BRUSH.codepoint, "Pena / Menggambar Bebas (P)"),
                    (CanvasTool::Eraser, ICON_DELETE.codepoint, "Penghapus (E)"),
                ];

                for (tool, icon, tooltip) in tools {
                    let is_active = active_tool == tool;
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());

                    if is_active {
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(ROUNDING_SM),
                            ACCENT_BLUE,
                            Stroke::new(1.0, ACCENT_BLUE),
                            StrokeKind::Inside,
                        );
                    } else if resp.hovered() {
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(ROUNDING_SM),
                            BG_HOVER_DARK,
                            Stroke::new(0.5, BORDER_SUBTLE),
                            StrokeKind::Inside,
                        );
                    }

                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        icon,
                        egui::FontId::proportional(icon_sz),
                        if is_active {
                            Color32::WHITE
                        } else {
                            TEXT_SECONDARY
                        },
                    );

                    if resp.on_hover_text(tooltip).clicked() {
                        event = Some(LeftToolbarEvent::SelectCanvasTool(tool));
                    }

                    ui.add_space(2.0);
                }

                ui.add_space(4.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(4.0);

                // Undo & Redo quick action buttons
                let (u_rect, u_resp) =
                    ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                if u_resp.hovered() && can_undo {
                    ui.painter().rect(
                        u_rect,
                        CornerRadius::same(ROUNDING_SM),
                        BG_HOVER_DARK,
                        Stroke::NONE,
                        StrokeKind::Inside,
                    );
                }
                ui.painter().text(
                    u_rect.center(),
                    Align2::CENTER_CENTER,
                    ICON_UNDO.codepoint,
                    egui::FontId::proportional(15.0),
                    if can_undo {
                        TEXT_PRIMARY
                    } else {
                        TEXT_MUTED
                    },
                );
                if u_resp.on_hover_text("Urungkan (⌘Z)").clicked() && can_undo {
                    event = Some(LeftToolbarEvent::Undo);
                }

                ui.add_space(2.0);

                let (r_rect, r_resp) =
                    ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                if r_resp.hovered() && can_redo {
                    ui.painter().rect(
                        r_rect,
                        CornerRadius::same(ROUNDING_SM),
                        BG_HOVER_DARK,
                        Stroke::NONE,
                        StrokeKind::Inside,
                    );
                }
                ui.painter().text(
                    r_rect.center(),
                    Align2::CENTER_CENTER,
                    ICON_REDO.codepoint,
                    egui::FontId::proportional(15.0),
                    if can_redo {
                        TEXT_PRIMARY
                    } else {
                        TEXT_MUTED
                    },
                );
                if r_resp.on_hover_text("Ulangi (⌘Shift+Z)").clicked() && can_redo {
                    event = Some(LeftToolbarEvent::Redo);
                }

                ui.add_space(2.0);
            });
        });

        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_left_toolbar_event_variants() {
        let ev = LeftToolbarEvent::SelectCanvasTool(CanvasTool::Select);
        assert_eq!(ev, LeftToolbarEvent::SelectCanvasTool(CanvasTool::Select));
    }
}
