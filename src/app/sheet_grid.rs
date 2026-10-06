//! Sheet grid (§3.8.2): a virtualized `egui_extras::TableBuilder` over the
//! visible rows (only rows on screen are laid out, so large CSVs stay
//! smooth), with in-place cell editing and spreadsheet keys:
//! arrows move, Enter/F2 edit, typing replaces, Enter/Tab commit and
//! advance, Esc cancels, Delete clears, ⌘Z/⇧⌘Z undo/redo, ⌘C copies the
//! cell and ⌘V pastes a tab-separated block (from Excel/Sheets) as one
//! undo step. Header click sorts; header and row-number context menus
//! insert/delete/rename. Callers: `app::sheet::show_sheet_viewer`.

use egui::{Align, Align2, Event, FontId, Id, Key, KeyboardShortcut, Layout, Modifiers, Sense};
use egui_extras::{Column, TableBuilder};
use egui_icons::icons::{
    ICON_ADD_ROW_ABOVE, ICON_ADD_ROW_BELOW, ICON_ARROW_DOWNWARD, ICON_ARROW_UPWARD, ICON_DELETE,
    ICON_EDIT, ICON_VIEW_COLUMN,
};

use super::MnemonicApp;
use super::sheet::SheetViewerState;
use crate::ui::{pal, theme, widgets};

const ROW_HEIGHT: f32 = 26.0;
const HEADER_HEIGHT: f32 = 30.0;
const ROW_NUMBER_WIDTH: f32 = 56.0;
const INITIAL_COLUMN_WIDTH: f32 = 150.0;
/// Characters of a cell drawn in the grid; the editor shows the rest.
const MAX_DRAWN_CHARS: usize = 200;
/// Rows jumped by PageUp/PageDown.
const PAGE_ROWS: isize = 20;

type Tr<'a> = &'a dyn Fn(&str) -> String;

impl MnemonicApp {
    pub(super) fn sheet_grid(&self, ui: &mut egui::Ui, viewer: &mut SheetViewerState) {
        let tr = &self.locales;
        let t = |key: &str| tr.t(key, &[]);
        let editable = viewer.doc.editable();
        let width = viewer.doc.sheet().width();

        if width == 0 {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(t("sheet-empty")).color(pal().text_dim));
                let icon = Some(ICON_VIEW_COLUMN.codepoint);
                if editable && widgets::secondary_button(ui, icon, &t("sheet-add-column")).clicked() {
                    viewer.add_column();
                }
            });
            return;
        }

        handle_keys(ui, viewer);
        viewer.refresh_view();

        let mut table = TableBuilder::new(ui)
            .id_salt(("sheet_grid", &viewer.path, viewer.doc.active))
            .striped(true)
            .resizable(true)
            .auto_shrink(false)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(ROW_NUMBER_WIDTH))
            .columns(
                Column::initial(INITIAL_COLUMN_WIDTH).at_least(48.0).clip(true),
                width,
            );
        if let Some(vi) = viewer.scroll_to.take() {
            table = table.scroll_to_row(vi, None);
        }

        table
            .header(HEADER_HEIGHT, |mut header| {
                header.col(|ui| {
                    let hash = egui::RichText::new("#").size(theme::TEXT_SM);
                    ui.label(hash.color(pal().text_faint));
                });
                for col in 0..width {
                    header.col(|ui| header_cell(ui, viewer, col, editable, &t));
                }
            })
            .body(|body| {
                let rows = viewer.visible.len();
                body.rows(ROW_HEIGHT, rows, |mut row| {
                    let Some(&r) = viewer.visible.get(row.index()) else {
                        return;
                    };
                    row.col(|ui| row_number_cell(ui, viewer, r, editable, &t));
                    for col in 0..width {
                        row.col(|ui| body_cell(ui, viewer, r, col));
                    }
                });
            });
    }
}

fn header_cell(ui: &mut egui::Ui, viewer: &mut SheetViewerState, col: usize, editable: bool, t: Tr) {
    let p = pal();
    if let Some((c, buffer, focused)) = viewer.renaming.as_mut()
        && *c == col
    {
        let resp = ui.add(
            egui::TextEdit::singleline(buffer)
                .id(Id::new("sheet_rename_column"))
                .desired_width(f32::INFINITY),
        );
        if !*focused {
            resp.request_focus();
            *focused = true;
        } else if resp.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                viewer.renaming = None;
            } else {
                viewer.commit_rename();
            }
        }
        return;
    }

    let Some(name) = viewer.doc.sheet().headers.get(col).cloned() else {
        return;
    };
    let arrow = match viewer.sort {
        Some((c, true)) if c == col => format!(" {}", ICON_ARROW_UPWARD.codepoint),
        Some((c, false)) if c == col => format!(" {}", ICON_ARROW_DOWNWARD.codepoint),
        _ => String::new(),
    };
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    let selected_col = viewer.cursor.is_some_and(|(_, c)| c == col);
    ui.painter().with_clip_rect(rect).text(
        rect.left_center() + egui::vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{name}{arrow}"),
        FontId::proportional(theme::TEXT_SM),
        if selected_col { p.accent } else { p.text },
    );
    let resp = resp.on_hover_text(t("sheet-sort-hint"));
    if resp.clicked() {
        viewer.toggle_sort(col);
    }
    resp.context_menu(|ui| {
        ui.set_min_width(200.0);
        let item = |ui: &mut egui::Ui, icon: &str, key: &str| {
            widgets::menu_item(ui, icon, &t(key), None).clicked()
        };
        if item(ui, ICON_ARROW_UPWARD.codepoint, "sheet-sort-asc") {
            viewer.set_sort(col, true);
            ui.close();
        }
        if item(ui, ICON_ARROW_DOWNWARD.codepoint, "sheet-sort-desc") {
            viewer.set_sort(col, false);
            ui.close();
        }
        if !editable {
            return;
        }
        ui.separator();
        if item(ui, ICON_EDIT.codepoint, "sheet-rename-column") {
            viewer.renaming = Some((col, name.clone(), false));
            ui.close();
        }
        if item(ui, ICON_VIEW_COLUMN.codepoint, "sheet-insert-column-left") {
            viewer.insert_column_at(col);
            ui.close();
        }
        if item(ui, ICON_VIEW_COLUMN.codepoint, "sheet-insert-column-right") {
            viewer.insert_column_at(col + 1);
            ui.close();
        }
        if item(ui, ICON_DELETE.codepoint, "sheet-delete-column") {
            viewer.doc.remove_column(col);
            if viewer.sort.is_some_and(|(c, _)| c == col) {
                viewer.sort = None;
            }
            viewer.invalidate();
            ui.close();
        }
    });
}

fn row_number_cell(
    ui: &mut egui::Ui,
    viewer: &mut SheetViewerState,
    row: usize,
    editable: bool,
    t: Tr,
) {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    let selected = viewer.cursor.is_some_and(|(r, _)| r == row);
    ui.painter().text(
        rect.right_center() - egui::vec2(8.0, 0.0),
        Align2::RIGHT_CENTER,
        (row + 1).to_string(),
        FontId::monospace(theme::TEXT_XS),
        if selected { p.accent } else { p.text_faint },
    );
    if resp.clicked() {
        viewer.commit_edit(0, 0);
        let col = viewer.cursor.map(|(_, c)| c).unwrap_or(0);
        viewer.cursor = Some((row, col));
    }
    if !editable {
        return;
    }
    resp.context_menu(|ui| {
        ui.set_min_width(200.0);
        let item = |ui: &mut egui::Ui, icon: &str, key: &str| {
            widgets::menu_item(ui, icon, &t(key), None).clicked()
        };
        if item(ui, ICON_ADD_ROW_ABOVE.codepoint, "sheet-insert-row-above") {
            viewer.insert_row_at(row);
            ui.close();
        }
        if item(ui, ICON_ADD_ROW_BELOW.codepoint, "sheet-insert-row-below") {
            viewer.insert_row_at(row + 1);
            ui.close();
        }
        if item(ui, ICON_DELETE.codepoint, "sheet-delete-row") {
            viewer.doc.remove_row(row);
            viewer.invalidate();
            ui.close();
        }
    });
}

fn body_cell(ui: &mut egui::Ui, viewer: &mut SheetViewerState, row: usize, col: usize) {
    let p = pal();
    if let Some(edit) = viewer.editing.as_mut()
        && edit.row == row
        && edit.col == col
    {
        let id = Id::new("sheet_cell_edit");
        let resp = ui.add(
            egui::TextEdit::singleline(&mut edit.buffer)
                .id(id)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY),
        );
        if edit.fresh {
            edit.fresh = false;
            resp.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let end = egui::text::CCursor::new(edit.buffer.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), id);
            }
        } else if resp.lost_focus() {
            let (esc, tab, enter, shift) = ui.input(|i| {
                (
                    i.key_pressed(Key::Escape),
                    i.key_pressed(Key::Tab),
                    i.key_pressed(Key::Enter),
                    i.modifiers.shift,
                )
            });
            let back = if shift { -1 } else { 1 };
            if esc {
                viewer.editing = None;
            } else if tab {
                viewer.commit_edit(0, back);
            } else if enter {
                viewer.commit_edit(back, 0);
            } else {
                viewer.commit_edit(0, 0);
            }
        }
        return;
    }

    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    if viewer.cursor == Some((row, col)) {
        ui.painter().rect(
            rect.shrink(1.0),
            2.0,
            p.accent_soft,
            egui::Stroke::new(1.5, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    let text = viewer.doc.sheet().cell(row, col).unwrap_or("");
    let truncated = text.contains('\n') || text.chars().count() > MAX_DRAWN_CHARS;
    if !text.is_empty() {
        let mut shown: String = text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(MAX_DRAWN_CHARS)
            .collect();
        if truncated {
            shown.push('…');
        }
        let numeric = viewer.numeric_cols.get(col).copied().unwrap_or(false);
        let (pos, align) = if numeric {
            (rect.right_center() - egui::vec2(6.0, 0.0), Align2::RIGHT_CENTER)
        } else {
            (rect.left_center() + egui::vec2(6.0, 0.0), Align2::LEFT_CENTER)
        };
        ui.painter().with_clip_rect(rect).text(
            pos,
            align,
            shown,
            FontId::proportional(theme::TEXT_SM),
            p.text,
        );
    }
    let full = truncated.then(|| text.to_string());
    if resp.double_clicked() {
        viewer.begin_edit(row, col, None);
    } else if resp.clicked() {
        viewer.commit_edit(0, 0);
        viewer.commit_rename();
        viewer.cursor = Some((row, col));
    }
    if let Some(full) = full {
        resp.on_hover_text(full);
    }
}

/// Spreadsheet keys while the grid (not a text field) has the keyboard.
fn handle_keys(ui: &mut egui::Ui, viewer: &mut SheetViewerState) {
    if viewer.has_dialog_open() || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let editable = viewer.doc.editable();
    let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
    let redo = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    let redo_alt = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
    // Shift+Cmd+Z first: `consume_shortcut` matches Cmd+Z loosely.
    let (redo_pressed, undo_pressed) = ui.input_mut(|i| {
        let r = i.consume_shortcut(&redo) || i.consume_shortcut(&redo_alt);
        (r, i.consume_shortcut(&undo))
    });
    if redo_pressed {
        viewer.redo();
    } else if undo_pressed {
        viewer.undo();
    }

    let events = ui.input(|i| i.events.clone());
    for event in events {
        match event {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if !modifiers.command => match key {
                Key::ArrowUp => viewer.move_cursor(-1, 0),
                Key::ArrowDown => viewer.move_cursor(1, 0),
                Key::ArrowLeft => viewer.move_cursor(0, -1),
                Key::ArrowRight => viewer.move_cursor(0, 1),
                Key::Tab => viewer.move_cursor(0, if modifiers.shift { -1 } else { 1 }),
                Key::PageUp => viewer.move_cursor(-PAGE_ROWS, 0),
                Key::PageDown => viewer.move_cursor(PAGE_ROWS, 0),
                Key::Enter | Key::F2 => {
                    if let Some((r, c)) = viewer.cursor {
                        viewer.begin_edit(r, c, None);
                        break;
                    }
                }
                Key::Delete | Key::Backspace if editable => {
                    if let Some((r, c)) = viewer.cursor {
                        viewer.doc.set_cell(r, c, String::new());
                        viewer.invalidate();
                    }
                }
                _ => {}
            },
            Event::Text(text) if editable && !text.chars().any(char::is_control) => {
                if let Some((r, c)) = viewer.cursor {
                    viewer.begin_edit(r, c, Some(text));
                    break;
                }
            }
            Event::Copy => {
                if let Some(text) = viewer
                    .cursor
                    .and_then(|(r, c)| viewer.doc.sheet().cell(r, c))
                {
                    ui.ctx().copy_text(text.to_string());
                }
            }
            Event::Paste(text) if editable => {
                if let Some((r, c)) = viewer.cursor {
                    if text.contains('\t') || text.trim_end().contains('\n') {
                        viewer.doc.paste_block(r, c, &text);
                    } else {
                        viewer.doc.set_cell(r, c, text);
                    }
                    viewer.invalidate();
                }
            }
            _ => {}
        }
    }
}
