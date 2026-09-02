//! Slide-over Glass Drawer / Sidebar bergaya Shapr3D / DUCAD.
//!
//! Menampilkan drawer mengambang di sisi kiri untuk navigasi filter dokumen,
//! tag label berwarna, dan perpustakaan dokumen PDF.

use std::path::PathBuf;

use egui::{
    Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};
use egui_icons::icons::{
    ICON_CATEGORY, ICON_CLOSE, ICON_DELETE, ICON_DESCRIPTION, ICON_DRAW, ICON_EDIT, ICON_FOLDER,
    ICON_INVENTORY_2, ICON_PICTURE_AS_PDF, ICON_UPLOAD,
};

use crate::ui::theme::{
    glass_panel_frame, tag_color, ACCENT_BLUE, BG_HOVER_DARK, BORDER_SUBTLE, ROUNDING_SM,
    SIDEBAR_WIDTH, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY, TOPBAR_HEIGHT,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarDocFilter {
    All,
    NotesOnly,
    WhiteboardsOnly,
    PdfsOnly,
    Archived,
    Trashed,
    Tag(String),
}

#[derive(Debug, Clone)]
pub enum SidebarEvent {
    SelectFilter(SidebarDocFilter),
    OpenPdf(PathBuf),
    ImportPdf,
    ManageLabels,
    CloseSidebar,
}

pub struct SidebarState {
    pub is_open: bool,
    pub current_filter: SidebarDocFilter,
    pub all_tags: Vec<(String, usize)>,
    pub pdf_documents: Vec<PathBuf>,
    pub total_notes: usize,
    pub total_pdfs: usize,
    pub total_whiteboards: usize,
    pub total_archived: usize,
    pub total_trashed: usize,
}

pub struct SidebarDrawer;

impl SidebarDrawer {
    /// Render slide-over sidebar drawer. Mengembalikan `Option<SidebarEvent>`.
    pub fn show(ctx: &egui::Context, state: &SidebarState) -> Option<SidebarEvent> {
        if !state.is_open {
            return None;
        }

        let mut event = None;
        let screen_rect = ctx.viewport_rect();
        let top_offset = TOPBAR_HEIGHT + 14.0;
        let sidebar_rect = Rect::from_min_size(
            screen_rect.min + Vec2::new(12.0, top_offset),
            Vec2::new(SIDEBAR_WIDTH, (screen_rect.height() - top_offset - 16.0).max(200.0)),
        );

        // Backdrop click to dismiss
        let backdrop_layer = egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("sidebar_backdrop_layer"),
        );
        let backdrop_painter = ctx.layer_painter(backdrop_layer);
        backdrop_painter.rect_filled(
            screen_rect,
            CornerRadius::ZERO,
            Color32::from_black_alpha(40),
        );

        egui::Window::new("sidebar_glass_drawer")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_rect(sidebar_rect)
            .frame(glass_panel_frame())
            .show(ctx, |ui| {
                ui.add_space(8.0);

                // Header Drawer
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(ICON_FOLDER.codepoint)
                            .size(16.0)
                            .color(ACCENT_BLUE),
                    );
                    ui.label(
                        RichText::new("Dokumen & Vault")
                            .size(13.5)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        let close_btn = egui::Button::new(
                            RichText::new(ICON_CLOSE.codepoint)
                                .size(14.0)
                                .color(TEXT_SECONDARY),
                        )
                        .frame(false);

                        if ui.add(close_btn).clicked() {
                            event = Some(SidebarEvent::CloseSidebar);
                        }
                    });
                });

                ui.add_space(6.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(6.0);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(2.0);

                    // ── Kategori Dokumen ──
                    let items = [
                        (
                            SidebarDocFilter::All,
                            ICON_CATEGORY.codepoint,
                            "Semua Dokumen",
                            state.total_notes + state.total_pdfs + state.total_whiteboards,
                        ),
                        (
                            SidebarDocFilter::NotesOnly,
                            ICON_DESCRIPTION.codepoint,
                            "Catatan Markdown",
                            state.total_notes,
                        ),
                        (
                            SidebarDocFilter::WhiteboardsOnly,
                            ICON_DRAW.codepoint,
                            "Whiteboard Kanvas",
                            state.total_whiteboards,
                        ),
                        (
                            SidebarDocFilter::PdfsOnly,
                            ICON_PICTURE_AS_PDF.codepoint,
                            "Dokumen PDF",
                            state.total_pdfs,
                        ),
                        (
                            SidebarDocFilter::Archived,
                            ICON_INVENTORY_2.codepoint,
                            "Arsip",
                            state.total_archived,
                        ),
                        (
                            SidebarDocFilter::Trashed,
                            ICON_DELETE.codepoint,
                            "Tong Sampah",
                            state.total_trashed,
                        ),
                    ];

                    for (filter_kind, icon, label, count) in items {
                        let is_active = state.current_filter == filter_kind;
                        let text_color = if is_active {
                            Color32::WHITE
                        } else {
                            TEXT_SECONDARY
                        };

                        let bg_fill = if is_active {
                            Color32::from_rgba_premultiplied(10, 132, 255, 60)
                        } else {
                            Color32::TRANSPARENT
                        };

                        let (rect, resp) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width() - 8.0, 28.0),
                            Sense::click(),
                        );

                        if resp.hovered() && !is_active {
                            ui.painter().rect(
                                rect,
                                CornerRadius::same(ROUNDING_SM),
                                BG_HOVER_DARK,
                                Stroke::new(0.5, BORDER_SUBTLE),
                                StrokeKind::Inside,
                            );
                        } else if is_active {
                            ui.painter().rect(
                                rect,
                                CornerRadius::same(ROUNDING_SM),
                                bg_fill,
                                Stroke::new(1.0, ACCENT_BLUE),
                                StrokeKind::Inside,
                            );
                        }

                        // Icon + Label
                        let icon_pos = Pos2::new(rect.min.x + 8.0, rect.center().y);
                        ui.painter().text(
                            icon_pos,
                            Align2::LEFT_CENTER,
                            icon,
                            egui::FontId::proportional(14.0),
                            if is_active { ACCENT_BLUE } else { text_color },
                        );

                        let text_pos = Pos2::new(rect.min.x + 28.0, rect.center().y);
                        ui.painter().text(
                            text_pos,
                            Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(12.5),
                            text_color,
                        );

                        // Count pill
                        let count_str = count.to_string();
                        let count_pos = Pos2::new(rect.max.x - 8.0, rect.center().y);
                        ui.painter().text(
                            count_pos,
                            Align2::RIGHT_CENTER,
                            &count_str,
                            egui::FontId::proportional(11.0),
                            TEXT_MUTED,
                        );

                        if resp.clicked() {
                            event = Some(SidebarEvent::SelectFilter(filter_kind));
                        }

                        ui.add_space(2.0);
                    }

                    ui.add_space(10.0);
                    ui.add(egui::Separator::default().spacing(0.0));
                    ui.add_space(6.0);

                    // ── Bagian Tag / Label ──
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("LABEL & TAG")
                                .size(10.5)
                                .color(TEXT_MUTED)
                                .strong(),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(6.0);
                            let manage_btn = egui::Button::new(
                                RichText::new(format!("{} Kelola", ICON_EDIT.codepoint))
                                    .size(10.5)
                                    .color(ACCENT_BLUE),
                            )
                            .frame(false);

                            if ui.add(manage_btn).clicked() {
                                event = Some(SidebarEvent::ManageLabels);
                            }
                        });
                    });

                    ui.add_space(4.0);

                    if state.all_tags.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new("Belum ada tag")
                                    .size(11.5)
                                    .color(TEXT_MUTED),
                            );
                        });
                    } else {
                        for (tag, count) in &state.all_tags {
                            let is_active = matches!(&state.current_filter, SidebarDocFilter::Tag(t) if t.eq_ignore_ascii_case(tag));
                            let color = tag_color(tag);

                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width() - 8.0, 26.0),
                                Sense::click(),
                            );

                            if resp.hovered() && !is_active {
                                ui.painter().rect(
                                    rect,
                                    CornerRadius::same(ROUNDING_SM),
                                    BG_HOVER_DARK,
                                    Stroke::NONE,
                                    StrokeKind::Inside,
                                );
                            } else if is_active {
                                ui.painter().rect(
                                    rect,
                                    CornerRadius::same(ROUNDING_SM),
                                    Color32::from_rgba_premultiplied(10, 132, 255, 40),
                                    Stroke::new(1.0, ACCENT_BLUE),
                                    StrokeKind::Inside,
                                );
                            }

                            // Dot warna tag
                            let dot_pos = Pos2::new(rect.min.x + 12.0, rect.center().y);
                            ui.painter().circle_filled(dot_pos, 4.0, color);

                            // Tag name
                            let tag_pos = Pos2::new(rect.min.x + 24.0, rect.center().y);
                            ui.painter().text(
                                tag_pos,
                                Align2::LEFT_CENTER,
                                format!("#{tag}"),
                                egui::FontId::proportional(12.0),
                                if is_active { Color32::WHITE } else { TEXT_SECONDARY },
                            );

                            // Count badge
                            let count_pos = Pos2::new(rect.max.x - 8.0, rect.center().y);
                            ui.painter().text(
                                count_pos,
                                Align2::RIGHT_CENTER,
                                count.to_string(),
                                egui::FontId::proportional(11.0),
                                TEXT_MUTED,
                            );

                            if resp.clicked() {
                                event = Some(SidebarEvent::SelectFilter(SidebarDocFilter::Tag(
                                    tag.clone(),
                                )));
                            }

                            ui.add_space(1.0);
                        }
                    }

                    ui.add_space(10.0);
                    ui.add(egui::Separator::default().spacing(0.0));
                    ui.add_space(6.0);

                    // ── Bagian PDF Ingestion ──
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("BERKAS PDF")
                                .size(10.5)
                                .color(TEXT_MUTED)
                                .strong(),
                        );
                    });

                    ui.add_space(4.0);

                    let import_btn = egui::Button::new(
                        RichText::new(format!("{}  Impor PDF ke Vault", ICON_UPLOAD.codepoint))
                            .size(12.0)
                            .color(Color32::WHITE),
                    )
                    .fill(ACCENT_BLUE)
                    .corner_radius(CornerRadius::same(ROUNDING_SM));

                    if ui
                        .add_sized(Vec2::new(ui.available_width() - 8.0, 28.0), import_btn)
                        .clicked()
                    {
                        event = Some(SidebarEvent::ImportPdf);
                    }

                    ui.add_space(4.0);

                    for path in &state.pdf_documents {
                        let file_name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Dokumen.pdf".to_string());

                        let truncated = if file_name.len() > 22 {
                            format!("{}…", &file_name[..19])
                        } else {
                            file_name.clone()
                        };

                        let pdf_btn = egui::Button::new(
                            RichText::new(format!(
                                "{}  {truncated}",
                                ICON_PICTURE_AS_PDF.codepoint
                            ))
                            .size(11.5)
                            .color(TEXT_SECONDARY),
                        )
                        .frame(false);

                        if ui.add(pdf_btn).clicked() {
                            event = Some(SidebarEvent::OpenPdf(path.clone()));
                        }
                    }

                    ui.add_space(12.0);
                });
            });

        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sidebar_state() {
        let state = SidebarState {
            is_open: true,
            current_filter: SidebarDocFilter::All,
            all_tags: vec![("rust".to_string(), 4), ("notes".to_string(), 12)],
            pdf_documents: vec![],
            total_notes: 16,
            total_pdfs: 0,
            total_whiteboards: 1,
            total_archived: 2,
            total_trashed: 0,
        };

        assert!(state.is_open);
        assert_eq!(state.current_filter, SidebarDocFilter::All);
        assert_eq!(state.all_tags.len(), 2);
    }
}
