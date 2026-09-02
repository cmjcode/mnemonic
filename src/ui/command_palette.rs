//! Command Palette gaya VS Code / Spotlight bergaya Shapr3D / DUCAD.
//!
//! Pemicu keyboard global (Cmd+K / Ctrl+K) atau klik bar pencarian atas.
//! Mendukung pencarian cepat aksi, navigasi keyboard (Up/Down/Enter/Esc),
//! dan eksekusi perintah dengan indikator hint shortcut.

use egui::{
    Color32, CornerRadius, Frame, Key, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use egui_icons::icons::ICON_SEARCH;

use crate::ui::theme::{
    glass_frame, ACCENT_BLUE, BG_HOVER_DARK, BORDER_SUBTLE, ROUNDING_SM, TEXT_MUTED, TEXT_PRIMARY,
};

#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub id: &'static str,
    pub category: &'static str,
    pub icon: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
}

#[derive(Default)]
pub struct CommandPalette {
    open: bool,
    query: String,
    highlighted: usize,
    focus_pending: bool,
}

impl CommandPalette {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.highlighted = 0;
        self.focus_pending = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// Render overlay command palette. Mengembalikan `Option<&'static str>` berisi command ID terpilih.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        commands: &[PaletteCommand],
    ) -> Option<&'static str> {
        if !self.open {
            return None;
        }

        let mut executed_id = None;
        let screen_rect = ctx.viewport_rect();

        // Backdrop gelap
        let backdrop_layer = egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("command_palette_backdrop"),
        );
        let backdrop_painter = ctx.layer_painter(backdrop_layer);
        backdrop_painter.rect_filled(
            screen_rect,
            CornerRadius::ZERO,
            Color32::from_black_alpha(110),
        );

        // Filter commands berdasarkan query
        let query_clean = self.query.trim().to_lowercase();
        let filtered_indices: Vec<usize> = commands
            .iter()
            .enumerate()
            .filter(|(_, cmd)| {
                query_clean.is_empty()
                    || cmd.label.to_lowercase().contains(&query_clean)
                    || cmd.category.to_lowercase().contains(&query_clean)
                    || cmd.hint.to_lowercase().contains(&query_clean)
            })
            .map(|(i, _)| i)
            .collect();

        if !filtered_indices.is_empty() {
            self.highlighted = self.highlighted.min(filtered_indices.len() - 1);
        } else {
            self.highlighted = 0;
        }

        // Keyboard handling (Escape, Up, Down, Enter)
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.close();
            return None;
        }

        if ctx.input(|i| i.key_pressed(Key::ArrowDown)) && !filtered_indices.is_empty() {
            self.highlighted = (self.highlighted + 1) % filtered_indices.len();
        }

        if ctx.input(|i| i.key_pressed(Key::ArrowUp)) && !filtered_indices.is_empty() {
            self.highlighted = if self.highlighted == 0 {
                filtered_indices.len() - 1
            } else {
                self.highlighted - 1
            };
        }

        let enter_pressed = ctx.input(|i| i.key_pressed(Key::Enter));
        if enter_pressed && !filtered_indices.is_empty() {
            let selected_cmd_idx = filtered_indices[self.highlighted];
            executed_id = Some(commands[selected_cmd_idx].id);
            self.close();
            return executed_id;
        }

        let modal_width: f32 = 540.0_f32.min(screen_rect.width() - 32.0);
        let modal_height: f32 = 360.0_f32.min(screen_rect.height() - 80.0);
        let modal_pos = Pos2::new(
            screen_rect.center().x - modal_width / 2.0,
            (screen_rect.min.y + 70.0).max(screen_rect.min.y + 20.0),
        );

        egui::Window::new("command_palette_modal")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_rect(Rect::from_min_size(modal_pos, Vec2::new(modal_width, modal_height)))
            .frame(glass_frame())
            .show(ctx, |ui| {
                ui.add_space(4.0);

                // Input Bar dengan Icon Search
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(ICON_SEARCH.codepoint)
                            .size(16.0)
                            .color(ACCENT_BLUE),
                    );

                    let input = egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Ketik perintah atau cari aksi... (Esc untuk batal)")
                        .desired_width(ui.available_width() - 16.0)
                        .frame(egui::Frame::NONE);

                    let edit_resp = ui.add(input);
                    if self.focus_pending {
                        edit_resp.request_focus();
                        self.focus_pending = false;
                    }
                });

                ui.add_space(6.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(6.0);

                // Daftar Command Terfilter
                egui::ScrollArea::vertical()
                    .max_height(modal_height - 60.0)
                    .show(ui, |ui| {
                        if filtered_indices.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(30.0);
                                ui.label(
                                    RichText::new("Tidak ada perintah yang cocok")
                                        .size(13.0)
                                        .color(TEXT_MUTED),
                                );
                            });
                        } else {
                            for (list_idx, &cmd_idx) in filtered_indices.iter().enumerate() {
                                let cmd = &commands[cmd_idx];
                                let is_highlighted = list_idx == self.highlighted;

                                let item_frame = Frame {
                                    inner_margin: Margin::symmetric(10, 6),
                                    outer_margin: Margin::symmetric(0, 1),
                                    corner_radius: CornerRadius::same(ROUNDING_SM),
                                    fill: if is_highlighted {
                                        Color32::from_rgba_premultiplied(10, 132, 255, 55)
                                    } else {
                                        Color32::TRANSPARENT
                                    },
                                    stroke: if is_highlighted {
                                        Stroke::new(1.0, ACCENT_BLUE)
                                    } else {
                                        Stroke::NONE
                                    },
                                    shadow: egui::Shadow::NONE,
                                };

                                let resp = item_frame
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            // Icon
                                            ui.label(
                                                RichText::new(cmd.icon)
                                                    .size(14.0)
                                                    .color(if is_highlighted {
                                                        Color32::WHITE
                                                    } else {
                                                        ACCENT_BLUE
                                                    }),
                                            );

                                            ui.add_space(4.0);

                                            // Category badge
                                            ui.label(
                                                RichText::new(cmd.category)
                                                    .size(10.5)
                                                    .color(TEXT_MUTED),
                                            );

                                            ui.label(
                                                RichText::new("›")
                                                    .size(11.0)
                                                    .color(TEXT_MUTED),
                                            );

                                            // Label
                                            ui.label(
                                                RichText::new(cmd.label)
                                                    .size(12.5)
                                                    .strong()
                                                    .color(if is_highlighted {
                                                        Color32::WHITE
                                                    } else {
                                                        TEXT_PRIMARY
                                                    }),
                                            );

                                            // Shortcut hint rata kanan
                                            if !cmd.hint.is_empty() {
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        ui.label(
                                                            RichText::new(cmd.hint)
                                                                .size(11.0)
                                                                .color(TEXT_MUTED),
                                                        );
                                                    },
                                                );
                                            }
                                        });
                                    })
                                    .response;

                                if resp.interact(Sense::click()).clicked() {
                                    executed_id = Some(cmd.id);
                                    self.close();
                                    break;
                                }
                            }
                        }
                    });
            });

        executed_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_palette_open_close() {
        let mut palette = CommandPalette::default();
        assert!(!palette.is_open());

        palette.open();
        assert!(palette.is_open());

        palette.toggle();
        assert!(!palette.is_open());
    }
}
