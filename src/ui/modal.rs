//! Modals, Alert Dialogs, and Toast Banners bergaya Shapr3D / DUCAD.
//!
//! Menyediakan dialog konfirmasi hapus permanen, modal pengelola label,
//! serta banner notifikasi kesalahan / status.

use egui::{
    Color32, CornerRadius, Pos2, Rect, RichText, Sense, Vec2,
};
use egui_icons::icons::{
    ICON_CHECK, ICON_CLOSE, ICON_DELETE, ICON_EDIT, ICON_PALETTE, ICON_WARNING,
};

use crate::ui::theme::{
    glass_frame, tag_color, ACCENT_BLUE, BG_CARD_DARK, GLASS_ERROR, ROUNDING_SM, TEXT_MUTED,
    TEXT_PRIMARY, TEXT_SECONDARY,
};

pub struct ConfirmModal;

impl ConfirmModal {
    /// Render alert konfirmasi hapus atau tindakan destruktif lainnya.
    pub fn show(
        ctx: &egui::Context,
        title: &str,
        message: &str,
        confirm_label: &str,
        is_destructive: bool,
    ) -> Option<bool> {
        let mut result = None;
        let screen_rect = ctx.viewport_rect();

        let backdrop_layer = egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("confirm_modal_backdrop"),
        );
        let backdrop_painter = ctx.layer_painter(backdrop_layer);
        backdrop_painter.rect_filled(
            screen_rect,
            CornerRadius::ZERO,
            Color32::from_black_alpha(120),
        );

        let modal_width = 380.0;
        let modal_height = 170.0;
        let modal_pos = Pos2::new(
            screen_rect.center().x - modal_width / 2.0,
            screen_rect.center().y - modal_height / 2.0,
        );

        egui::Window::new("confirm_modal_dialog")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_rect(Rect::from_min_size(
                modal_pos,
                Vec2::new(modal_width, modal_height),
            ))
            .frame(glass_frame())
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let icon_color = if is_destructive {
                        GLASS_ERROR
                    } else {
                        ACCENT_BLUE
                    };
                    ui.label(
                        RichText::new(ICON_WARNING.codepoint)
                            .size(20.0)
                            .color(icon_color),
                    );
                    ui.label(RichText::new(title).size(14.0).strong().color(TEXT_PRIMARY));
                });

                ui.add_space(8.0);
                ui.label(RichText::new(message).size(12.5).color(TEXT_SECONDARY));

                ui.add_space(16.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let btn_color = if is_destructive {
                        GLASS_ERROR
                    } else {
                        ACCENT_BLUE
                    };
                    let confirm_btn = egui::Button::new(
                        RichText::new(confirm_label)
                            .size(12.5)
                            .color(Color32::WHITE),
                    )
                    .fill(btn_color)
                    .corner_radius(CornerRadius::same(ROUNDING_SM));

                    if ui.add(confirm_btn).clicked() {
                        result = Some(true);
                    }

                    ui.add_space(8.0);

                    let cancel_btn = egui::Button::new(
                        RichText::new("Batal")
                            .size(12.5)
                            .color(TEXT_SECONDARY),
                    )
                    .fill(BG_CARD_DARK)
                    .corner_radius(CornerRadius::same(ROUNDING_SM));

                    if ui.add(cancel_btn).clicked() {
                        result = Some(false);
                    }
                });
            });

        result
    }
}

pub struct LabelManagerModal;

impl LabelManagerModal {
    /// Render modal pengelolaan label tag (rename, hapus).
    pub fn show(
        ctx: &egui::Context,
        all_tags: &[(String, usize)],
        rename_input: &mut String,
        selected_tag_to_rename: &mut Option<String>,
    ) -> Option<LabelManagerEvent> {
        let mut event = None;
        let screen_rect = ctx.viewport_rect();

        let backdrop_layer = egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("label_modal_backdrop"),
        );
        let backdrop_painter = ctx.layer_painter(backdrop_layer);
        backdrop_painter.rect_filled(
            screen_rect,
            CornerRadius::ZERO,
            Color32::from_black_alpha(120),
        );

        let modal_width = 440.0;
        let modal_height = 380.0;
        let modal_pos = Pos2::new(
            screen_rect.center().x - modal_width / 2.0,
            screen_rect.center().y - modal_height / 2.0,
        );

        egui::Window::new("label_manager_modal")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_rect(Rect::from_min_size(
                modal_pos,
                Vec2::new(modal_width, modal_height),
            ))
            .frame(glass_frame())
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(ICON_PALETTE.codepoint)
                            .size(16.0)
                            .color(ACCENT_BLUE),
                    );
                    ui.label(
                        RichText::new("Kelola Label & Tag")
                            .size(14.0)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close_btn = egui::Button::new(
                            RichText::new(ICON_CLOSE.codepoint)
                                .size(14.0)
                                .color(TEXT_SECONDARY),
                        )
                        .frame(false);
                        if ui.add(close_btn).clicked() {
                            event = Some(LabelManagerEvent::Close);
                        }
                    });
                });

                ui.add_space(6.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(6.0);

                if all_tags.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(
                            RichText::new("Belum ada tag pada catatan di vault ini")
                                .size(12.5)
                                .color(TEXT_MUTED),
                        );
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for (tag, count) in all_tags {
                                let is_editing = selected_tag_to_rename
                                    .as_ref()
                                    .map(|t| t == tag)
                                    .unwrap_or(false);

                                ui.horizontal(|ui| {
                                    let dot_color = tag_color(tag);
                                    let (d_rect, _) =
                                        ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                                    ui.painter().circle_filled(d_rect.center(), 4.0, dot_color);

                                    if is_editing {
                                        let edit =
                                            egui::TextEdit::singleline(rename_input).desired_width(160.0);
                                        ui.add(edit);

                                        let save_btn = egui::Button::new(
                                            RichText::new(ICON_CHECK.codepoint)
                                                .size(13.0)
                                                .color(Color32::WHITE),
                                        )
                                        .fill(ACCENT_BLUE)
                                        .corner_radius(CornerRadius::same(ROUNDING_SM));

                                        if ui.add(save_btn).clicked() && !rename_input.trim().is_empty() {
                                            event = Some(LabelManagerEvent::Rename {
                                                old_tag: tag.clone(),
                                                new_tag: rename_input.trim().to_string(),
                                            });
                                            *selected_tag_to_rename = None;
                                        }

                                        let cancel_btn = egui::Button::new(
                                            RichText::new(ICON_CLOSE.codepoint)
                                                .size(13.0)
                                                .color(TEXT_SECONDARY),
                                        )
                                        .frame(false);

                                        if ui.add(cancel_btn).clicked() {
                                            *selected_tag_to_rename = None;
                                        }
                                    } else {
                                        ui.label(
                                            RichText::new(format!("#{tag}"))
                                                .size(12.5)
                                                .color(TEXT_PRIMARY),
                                        );
                                        ui.label(
                                            RichText::new(format!("({count})"))
                                                .size(11.0)
                                                .color(TEXT_MUTED),
                                        );

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let del_btn = egui::Button::new(
                                                    RichText::new(ICON_DELETE.codepoint)
                                                        .size(13.0)
                                                        .color(GLASS_ERROR),
                                                )
                                                .frame(false);

                                                if ui.add(del_btn).clicked() {
                                                    event = Some(LabelManagerEvent::Delete(
                                                        tag.clone(),
                                                    ));
                                                }

                                                let edit_btn = egui::Button::new(
                                                    RichText::new(ICON_EDIT.codepoint)
                                                        .size(13.0)
                                                        .color(TEXT_SECONDARY),
                                                )
                                                .frame(false);

                                                if ui.add(edit_btn).clicked() {
                                                    *selected_tag_to_rename = Some(tag.clone());
                                                    *rename_input = tag.clone();
                                                }
                                            },
                                        );
                                    }
                                });

                                ui.add_space(3.0);
                            }
                        });
                }
            });

        event
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelManagerEvent {
    Rename { old_tag: String, new_tag: String },
    Delete(String),
    Close,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_label_manager_event() {
        let ev = LabelManagerEvent::Rename {
            old_tag: "old".to_string(),
            new_tag: "new".to_string(),
        };
        assert_eq!(
            ev,
            LabelManagerEvent::Rename {
                old_tag: "old".to_string(),
                new_tag: "new".to_string()
            }
        );
    }
}
