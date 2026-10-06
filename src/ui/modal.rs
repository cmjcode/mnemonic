//! Modal dialogs: confirmation, text prompt, folder picker, label manager,
//! and the keyboard shortcut cheat sheet. All share one look — dimmed
//! backdrop, centered card, Esc / click-outside to cancel, Enter to confirm.

use std::path::PathBuf;

use egui::{Align, Align2, Id, Layout, Margin, RichText, Ui, Vec2};
use egui_icons::icons::{
    ICON_CHECK, ICON_CLOSE, ICON_DELETE, ICON_EDIT, ICON_FOLDER, ICON_HOME, ICON_KEYBOARD,
    ICON_LABEL, ICON_WARNING,
};

use crate::i18n::LocaleManager;
use crate::ui::theme::{self, pal, tag_color};
use crate::ui::widgets::{self, ButtonKind};

/// Shows a centered modal card of `width`. Returns `(content result,
/// dismissed)` where `dismissed` is true on Esc or a backdrop click.
fn modal<R>(
    ctx: &egui::Context,
    id: &str,
    width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> (Option<R>, bool) {
    let backdrop_clicked = widgets::modal_backdrop(ctx, Id::new((id, "backdrop")));
    let width = width.min(ctx.viewport_rect().width() - 32.0);
    let inner = egui::Window::new(id)
        .id(Id::new((id, "window")))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(width)
        .min_width(width)
        .max_width(width)
        .frame(theme::popover_frame().inner_margin(Margin::same(20)))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_width(width);
            add_contents(ui)
        })
        .and_then(|r| r.inner);
    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    (inner, esc || backdrop_clicked)
}

fn modal_title(ui: &mut Ui, icon: &str, icon_color: egui::Color32, title: &str) {
    let p = pal();
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(20.0).color(icon_color));
        ui.add_space(2.0);
        ui.label(
            RichText::new(title)
                .font(theme::semibold(theme::TEXT_LG))
                .color(p.text),
        );
    });
}

/// Right-aligned footer with a cancel and a confirm button. Returns
/// `Some(true)` for confirm, `Some(false)` for cancel.
fn footer(
    ui: &mut Ui,
    confirm_label: &str,
    cancel_label: &str,
    confirm_kind: ButtonKind,
    confirm_enabled: bool,
) -> Option<bool> {
    let mut result = None;
    ui.add_space(theme::SPACE_L);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.add_enabled_ui(confirm_enabled, |ui| {
            if widgets::button(ui, confirm_kind, None, confirm_label).clicked() {
                result = Some(true);
            }
        });
        if widgets::button(ui, ButtonKind::Ghost, None, cancel_label).clicked() {
            result = Some(false);
        }
    });
    result
}

pub struct ConfirmModal;

impl ConfirmModal {
    /// `Some(true)` confirmed, `Some(false)` cancelled, `None` still open.
    /// Enter confirms only non-destructive dialogs.
    pub fn show(
        ctx: &egui::Context,
        title: &str,
        message: &str,
        confirm_label: &str,
        cancel_label: &str,
        is_destructive: bool,
    ) -> Option<bool> {
        let p = pal();
        let (result, dismissed) = modal(ctx, "confirm_modal", 400.0, |ui| {
            let (icon, color) = if is_destructive {
                (ICON_WARNING.codepoint, p.danger)
            } else {
                (ICON_CHECK.codepoint, p.accent)
            };
            modal_title(ui, icon, color, title);
            ui.add_space(theme::SPACE_S);
            ui.label(
                RichText::new(message)
                    .size(theme::TEXT_BODY)
                    .color(p.text_dim),
            );
            let kind = if is_destructive {
                ButtonKind::Danger
            } else {
                ButtonKind::Primary
            };
            footer(ui, confirm_label, cancel_label, kind, true)
        });
        if dismissed {
            return Some(false);
        }
        if !is_destructive && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            return Some(true);
        }
        result.flatten()
    }
}

/// What the user chose when the note on disk changed under an unsaved
/// editor (§6 "Watcher Conflict").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    /// Discard the editor's version and take what's on disk.
    Reload,
    /// Write the editor's version over the disk version.
    Overwrite,
    /// Keep both: the editor's version as a new `(conflict)` note.
    SaveCopy,
    /// Decide later (the editor stays open, unsaved).
    Cancel,
}

pub struct ConflictModal;

impl ConflictModal {
    /// `None` while still open.
    pub fn show(ctx: &egui::Context, tr: &LocaleManager, file_name: &str) -> Option<ConflictChoice> {
        let p = pal();
        let (result, dismissed) = modal(ctx, "conflict_modal", 440.0, |ui| {
            modal_title(ui, ICON_WARNING.codepoint, p.warning, &tr.t("conflict-title", &[]));
            ui.add_space(theme::SPACE_S);
            ui.label(
                RichText::new(tr.t("conflict-body", &[("name", file_name)]))
                    .size(theme::TEXT_BODY)
                    .color(p.text_dim),
            );
            ui.add_space(theme::SPACE_L);
            let mut choice = None;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::button(ui, ButtonKind::Primary, None, &tr.t("conflict-reload", &[])).clicked() {
                    choice = Some(ConflictChoice::Reload);
                }
                if widgets::button(ui, ButtonKind::Danger, None, &tr.t("conflict-overwrite", &[])).clicked() {
                    choice = Some(ConflictChoice::Overwrite);
                }
                if widgets::button(ui, ButtonKind::Ghost, None, &tr.t("conflict-copy", &[])).clicked() {
                    choice = Some(ConflictChoice::SaveCopy);
                }
                if widgets::button(ui, ButtonKind::Ghost, None, &tr.t("confirm-cancel", &[])).clicked() {
                    choice = Some(ConflictChoice::Cancel);
                }
            });
            choice
        });
        if dismissed {
            return Some(ConflictChoice::Cancel);
        }
        result.flatten()
    }
}

pub struct PromptInputModal;

impl PromptInputModal {
    /// Text prompt (new folder, rename). `Some(true)` confirmed with a
    /// non-empty value, `Some(false)` cancelled, `None` still open.
    pub fn show(
        ctx: &egui::Context,
        title: &str,
        message: &str,
        input_value: &mut String,
        placeholder: &str,
        confirm_label: &str,
        cancel_label: &str,
    ) -> Option<bool> {
        let p = pal();
        let focused_once = Id::new("prompt_modal_focused");
        let (result, dismissed) = modal(ctx, "prompt_modal", 420.0, |ui| {
            modal_title(ui, ICON_EDIT.codepoint, p.accent, title);
            ui.add_space(theme::SPACE_S);
            ui.label(
                RichText::new(message)
                    .size(theme::TEXT_BODY)
                    .color(p.text_dim),
            );
            ui.add_space(theme::SPACE_M);

            let edit_id = Id::new("prompt_modal_input");
            let resp = ui.add(
                egui::TextEdit::singleline(input_value)
                    .id(edit_id)
                    .hint_text(placeholder)
                    .margin(Margin::symmetric(10, 8))
                    .desired_width(f32::INFINITY),
            );
            // Focus once when opened, selecting the existing text so the
            // user can type a replacement straight away.
            if !ui
                .ctx()
                .data(|d| d.get_temp::<bool>(focused_once))
                .unwrap_or(false)
            {
                resp.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
                    let len = input_value.chars().count();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(len),
                        )));
                    state.store(ui.ctx(), edit_id);
                }
                ui.ctx().data_mut(|d| d.insert_temp(focused_once, true));
            }

            let valid = !input_value.trim().is_empty();
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let clicked = footer(ui, confirm_label, cancel_label, ButtonKind::Primary, valid);
            if enter && valid { Some(true) } else { clicked }
        });
        let outcome = if dismissed {
            Some(false)
        } else {
            result.flatten()
        };
        if outcome.is_some() {
            ctx.data_mut(|d| d.remove::<bool>(focused_once));
        }
        outcome
    }
}

/// What the user picked in [`MoveFolderModal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveChoice {
    Root,
    Folder(PathBuf),
    Cancel,
}

pub struct MoveFolderModal;

impl MoveFolderModal {
    /// Folder picker for moving a file/folder. `None` while still open.
    pub fn show(
        ctx: &egui::Context,
        tr: &LocaleManager,
        item_name: &str,
        available_folders: &[(PathBuf, String)],
        search_filter: &mut String,
    ) -> Option<MoveChoice> {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let (result, dismissed) = modal(ctx, "move_modal", 440.0, |ui| {
            let mut choice = None;
            modal_title(
                ui,
                ICON_FOLDER.codepoint,
                p.accent,
                &tr.t("move-modal-title", &[("name", item_name)]),
            );
            ui.add_space(theme::SPACE_M);
            let width = ui.available_width();
            widgets::search_field(
                ui,
                Id::new("move_modal_search"),
                search_filter,
                &t("move-modal-search"),
                width,
                None,
            );
            ui.add_space(theme::SPACE_S);

            let filter = search_filter.to_lowercase();
            egui::ScrollArea::vertical()
                .max_height(280.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let row = |ui: &mut Ui, icon: &str, color, label: &str| {
                        widgets::list_row(
                            ui,
                            widgets::RowSpec {
                                icon,
                                icon_color: color,
                                label,
                                trailing: None,
                                selected: false,
                                indent: 0.0,
                                reserve_right: 0.0,
                            },
                        )
                        .clicked()
                    };
                    let root_label = t("move-modal-root");
                    if (filter.is_empty() || root_label.to_lowercase().contains(&filter))
                        && row(ui, ICON_HOME.codepoint, p.accent, &root_label)
                    {
                        choice = Some(MoveChoice::Root);
                    }
                    for (dir_path, display_name) in available_folders {
                        if !filter.is_empty() && !display_name.to_lowercase().contains(&filter) {
                            continue;
                        }
                        if row(ui, ICON_FOLDER.codepoint, p.folder_icon, display_name) {
                            choice = Some(MoveChoice::Folder(dir_path.clone()));
                        }
                    }
                });

            ui.add_space(theme::SPACE_M);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::ghost_button(ui, None, &t("confirm-cancel")).clicked() {
                    choice = Some(MoveChoice::Cancel);
                }
            });
            choice
        });
        if dismissed {
            return Some(MoveChoice::Cancel);
        }
        result.flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelManagerEvent {
    Rename { old_tag: String, new_tag: String },
    Delete(String),
    Close,
}

pub struct LabelManagerModal;

impl LabelManagerModal {
    /// Rename / delete tags across the vault.
    pub fn show(
        ctx: &egui::Context,
        tr: &LocaleManager,
        all_tags: &[(String, usize)],
        rename_input: &mut String,
        selected_tag_to_rename: &mut Option<String>,
    ) -> Option<LabelManagerEvent> {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let (result, dismissed) = modal(ctx, "label_manager_modal", 460.0, |ui| {
            let mut event = None;
            ui.horizontal(|ui| {
                modal_title(ui, ICON_LABEL.codepoint, p.accent, &t("tag-manager-title"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(
                        ui,
                        ICON_CLOSE.codepoint,
                        &t("pdf-metadata-close"),
                        false,
                    )
                    .clicked()
                    {
                        event = Some(LabelManagerEvent::Close);
                    }
                });
            });
            ui.add_space(theme::SPACE_M);

            if all_tags.is_empty() {
                ui.label(
                    RichText::new(t("tag-manager-empty"))
                        .size(theme::TEXT_BODY)
                        .color(p.text_dim),
                );
                return event;
            }

            egui::ScrollArea::vertical()
                .max_height(340.0)
                .show(ui, |ui| {
                    for (tag, count) in all_tags {
                        let editing = selected_tag_to_rename.as_deref() == Some(tag.as_str());
                        ui.horizontal(|ui| {
                            ui.set_height(theme::CONTROL_HEIGHT + 4.0);
                            let (dot, _) =
                                ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
                            ui.painter()
                                .circle_filled(dot.center(), 5.0, tag_color(tag));

                            if editing {
                                let resp = ui.add(
                                    egui::TextEdit::singleline(rename_input)
                                        .margin(Margin::symmetric(8, 5))
                                        .desired_width(ui.available_width() - 80.0),
                                );
                                resp.request_focus();
                                let enter = resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if widgets::icon_button(
                                    ui,
                                    ICON_CHECK.codepoint,
                                    &t("tag-rename"),
                                    true,
                                )
                                .clicked()
                                    || enter
                                {
                                    if !rename_input.trim().is_empty() {
                                        event = Some(LabelManagerEvent::Rename {
                                            old_tag: tag.clone(),
                                            new_tag: rename_input.trim().to_string(),
                                        });
                                    }
                                    *selected_tag_to_rename = None;
                                }
                                if widgets::icon_button(
                                    ui,
                                    ICON_CLOSE.codepoint,
                                    &t("confirm-cancel"),
                                    false,
                                )
                                .clicked()
                                {
                                    *selected_tag_to_rename = None;
                                }
                            } else {
                                ui.label(
                                    RichText::new(format!("#{tag}"))
                                        .size(theme::TEXT_BODY)
                                        .color(p.text),
                                );
                                ui.label(
                                    RichText::new(
                                        tr.t("tag-note-count", &[("count", &count.to_string())]),
                                    )
                                    .size(theme::TEXT_XS)
                                    .color(p.text_faint),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if widgets::icon_button(
                                        ui,
                                        ICON_DELETE.codepoint,
                                        &t("tag-delete"),
                                        false,
                                    )
                                    .clicked()
                                    {
                                        event = Some(LabelManagerEvent::Delete(tag.clone()));
                                    }
                                    if widgets::icon_button(
                                        ui,
                                        ICON_EDIT.codepoint,
                                        &t("tag-rename"),
                                        false,
                                    )
                                    .clicked()
                                    {
                                        *selected_tag_to_rename = Some(tag.clone());
                                        *rename_input = tag.clone();
                                    }
                                });
                            }
                        });
                    }
                });
            event
        });
        if dismissed {
            // Esc while renaming only cancels the rename, not the dialog.
            if selected_tag_to_rename.take().is_none() {
                return Some(LabelManagerEvent::Close);
            }
            return None;
        }
        result.flatten()
    }
}

pub struct ShortcutsModal;

impl ShortcutsModal {
    /// Keyboard shortcut cheat sheet: `chords` are `(glyphs, locale key)`
    /// for the configurable actions (from `config.toml`), followed by the
    /// fixed editing keys. Returns true when closed.
    pub fn show(ctx: &egui::Context, tr: &LocaleManager, chords: &[(String, &str)]) -> bool {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let mut rows: Vec<(String, &str)> = chords.to_vec();
        rows.extend([
            ("Esc".to_string(), "shortcut-back"),
            ("/".to_string(), "shortcut-slash"),
            ("[[".to_string(), "shortcut-wikilink"),
        ]);
        let (result, dismissed) = modal(ctx, "shortcuts_modal", 440.0, |ui| {
            let mut close = false;
            ui.horizontal(|ui| {
                modal_title(
                    ui,
                    ICON_KEYBOARD.codepoint,
                    p.accent,
                    &t("settings-shortcuts"),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close = widgets::icon_button(
                        ui,
                        ICON_CLOSE.codepoint,
                        &t("pdf-metadata-close"),
                        false,
                    )
                    .clicked();
                });
            });
            ui.add_space(theme::SPACE_M);
            egui::Grid::new("shortcuts_grid")
                .num_columns(2)
                .spacing(Vec2::new(24.0, 10.0))
                .show(ui, |ui| {
                    for (keys, label) in &rows {
                        ui.label(RichText::new(t(label)).size(theme::TEXT_BODY).color(p.text));
                        egui::Frame::NONE
                            .fill(p.surface)
                            .stroke(egui::Stroke::new(1.0, p.border))
                            .corner_radius(egui::CornerRadius::same(theme::RADIUS_SM))
                            .inner_margin(Margin::symmetric(8, 2))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(keys).size(theme::TEXT_SM).color(p.text_dim),
                                );
                            });
                        ui.end_row();
                    }
                });
            close
        });
        dismissed || result.unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_events_compare_by_value() {
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
        assert_ne!(MoveChoice::Root, MoveChoice::Cancel);
    }
}
