//! Command palette (⌘K): one keyboard-driven box to run any command or
//! jump straight to a note by title. ↑/↓ to move, Enter to run, Esc to close.

use egui::{Align2, CornerRadius, FontId, Id, Margin, Pos2, RichText, Sense, Vec2};

use crate::i18n::LocaleManager;
use crate::ui::theme::{self, pal};
use crate::ui::widgets;

#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub id: String,
    pub category: String,
    pub icon: &'static str,
    pub label: String,
    pub hint: String,
}

impl PaletteCommand {
    pub fn new(
        id: impl Into<String>,
        category: impl Into<String>,
        icon: &'static str,
        label: impl Into<String>,
    ) -> Self {
        PaletteCommand {
            id: id.into(),
            category: category.into(),
            icon,
            label: label.into(),
            hint: String::new(),
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }
}

/// Ranks `commands` against `query`: label prefix matches first, then
/// label substring, then category substring. Empty query keeps order.
pub fn filter_commands(commands: &[PaletteCommand], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..commands.len()).collect();
    }
    let mut scored: Vec<(u8, usize)> = commands
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let label = c.label.to_lowercase();
            if label.starts_with(&q) {
                Some((0, i))
            } else if label.contains(&q) {
                Some((1, i))
            } else if c.category.to_lowercase().contains(&q) {
                Some((2, i))
            } else {
                None
            }
        })
        .collect();
    scored.sort();
    scored.into_iter().map(|(_, i)| i).collect()
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

    pub fn query(&self) -> &str {
        &self.query
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

    /// Renders the palette; returns the id of the command chosen this frame.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        tr: &LocaleManager,
        commands: &[PaletteCommand],
    ) -> Option<String> {
        if !self.open {
            return None;
        }
        let p = pal();
        let t = |key: &str| tr.t(key, &[]);

        if widgets::modal_backdrop(ctx, Id::new("command_palette_backdrop"))
            || ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.close();
            return None;
        }

        let filtered = filter_commands(commands, &self.query);
        self.highlighted = self.highlighted.min(filtered.len().saturating_sub(1));

        let (down, up, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        let mut keyboard_moved = false;
        if !filtered.is_empty() {
            if down {
                self.highlighted = (self.highlighted + 1) % filtered.len();
                keyboard_moved = true;
            }
            if up {
                self.highlighted = (self.highlighted + filtered.len() - 1) % filtered.len();
                keyboard_moved = true;
            }
            if enter {
                self.close();
                return Some(commands[filtered[self.highlighted]].id.clone());
            }
        }

        let screen = ctx.viewport_rect();
        let width = 600.0_f32.min(screen.width() - 32.0);
        let mut executed = None;

        egui::Window::new("command_palette")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .order(egui::Order::Foreground)
            .anchor(
                Align2::CENTER_TOP,
                Vec2::new(0.0, (screen.height() * 0.14).max(40.0)),
            )
            .default_width(width)
            .min_width(width)
            .max_width(width)
            .frame(theme::popover_frame().inner_margin(Margin::same(8)))
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(egui_icons::icons::ICON_SEARCH.codepoint)
                            .size(20.0)
                            .color(p.text_faint),
                    );
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text(RichText::new(t("command-palette-hint")).color(p.text_faint))
                            .font(FontId::proportional(theme::TEXT_LG))
                            .frame(egui::Frame::NONE)
                            .margin(Margin::symmetric(4, 10))
                            .desired_width(f32::INFINITY),
                    );
                    if self.focus_pending {
                        resp.request_focus();
                        self.focus_pending = false;
                    }
                    if resp.changed() {
                        self.highlighted = 0;
                    }
                });
                ui.add(egui::Separator::default().spacing(8.0));

                egui::ScrollArea::vertical()
                    .max_height((screen.height() * 0.5).clamp(200.0, 440.0))
                    .min_scrolled_height((filtered.len().max(1) as f32 * 40.0).min(320.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if filtered.is_empty() {
                            ui.add_space(24.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(t("command-palette-empty"))
                                        .size(theme::TEXT_BODY)
                                        .color(p.text_faint),
                                );
                            });
                            ui.add_space(24.0);
                            return;
                        }
                        let grouped = self.query.trim().is_empty();
                        let mut last_category: Option<&str> = None;
                        for (list_idx, &cmd_idx) in filtered.iter().enumerate() {
                            let cmd = &commands[cmd_idx];
                            if grouped && last_category != Some(cmd.category.as_str()) {
                                widgets::section_header(ui, &cmd.category);
                                last_category = Some(cmd.category.as_str());
                            }
                            let highlighted = list_idx == self.highlighted;
                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 38.0),
                                Sense::click(),
                            );
                            if resp.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                                self.highlighted = list_idx;
                            }
                            if highlighted {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(theme::RADIUS_MD),
                                    p.accent_soft,
                                );
                                if keyboard_moved {
                                    resp.scroll_to_me(None);
                                }
                            }
                            ui.painter().text(
                                Pos2::new(rect.min.x + 14.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                cmd.icon,
                                FontId::proportional(18.0),
                                if highlighted { p.accent } else { p.text_dim },
                            );
                            let mut right = rect.max.x - 12.0;
                            if !cmd.hint.is_empty() {
                                let g = ui.painter().layout_no_wrap(
                                    cmd.hint.clone(),
                                    FontId::proportional(theme::TEXT_XS),
                                    p.text_faint,
                                );
                                right -= g.size().x;
                                ui.painter().galley(
                                    Pos2::new(right, rect.center().y - g.size().y / 2.0),
                                    g,
                                    p.text_faint,
                                );
                                right -= 12.0;
                            }
                            let label = widgets::elided_galley(
                                ui,
                                &cmd.label,
                                FontId::proportional(theme::TEXT_BODY),
                                p.text,
                                right - rect.min.x - 46.0,
                            );
                            ui.painter().galley(
                                Pos2::new(
                                    rect.min.x + 44.0,
                                    rect.center().y - label.size().y / 2.0,
                                ),
                                label,
                                p.text,
                            );
                            if resp
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                executed = Some(cmd.id.clone());
                            }
                        }
                    });

                ui.add(egui::Separator::default().spacing(8.0));
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(t("command-palette-footer"))
                            .size(theme::TEXT_XS)
                            .color(p.text_faint),
                    );
                });
            });

        if executed.is_some() {
            self.close();
        }
        executed
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

    #[test]
    fn filter_ranks_prefix_matches_first() {
        let cmds = vec![
            PaletteCommand::new("a", "Navigasi", "", "Buka catatan harian"),
            PaletteCommand::new("b", "Catatan", "", "Catatan Baru"),
            PaletteCommand::new("c", "Tampilan", "", "Ganti Tema"),
        ];
        assert_eq!(filter_commands(&cmds, "catatan"), vec![1, 0]);
        assert_eq!(filter_commands(&cmds, "tampil"), vec![2]);
        assert_eq!(filter_commands(&cmds, ""), vec![0, 1, 2]);
        assert!(filter_commands(&cmds, "zzz").is_empty());
    }
}
