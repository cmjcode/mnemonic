//! Source-mode Markdown editor (the whole body as styled raw text, reached
//! from the command palette / ⌘E) and the `/` slash-command +
//! `[[wikilink]]` autocomplete popup it shares with the Live line editor
//! (§3.2.1, §3.2.4). Callers: `app::editor`, `app::editor::live`.

use std::path::PathBuf;

use egui::{FontId, Frame, Id, Margin, Modifiers, RichText, Vec2};
use egui_icons::icons::ICON_LINK;

use super::EditorUi;
use crate::i18n::LocaleManager;
use crate::markdown::editor::{
    char_index_to_byte_offset, slash_menu_triggered, slash_templates, wikilink_autocomplete_query,
};
use crate::markdown::highlight::HighlightStyle;
use crate::markdown::{MarkdownEditor, WikilinkIndex};
use crate::notes::Vault;
use crate::ui::{pal, theme, widgets};

enum Completion {
    Slash(&'static str),
    Wikilink { query: String, title: String },
}

/// Popup navigation keys, taken from the input before the `TextEdit`
/// sees them (only while the popup is showing).
#[derive(Default)]
pub(super) struct PopupKeys {
    down: bool,
    up: bool,
    accept: bool,
    dismiss: bool,
}

pub(super) fn take_popup_keys(ctx: &egui::Context, state: &EditorUi) -> PopupKeys {
    let mut keys = PopupKeys::default();
    if state.popup_visible {
        ctx.input_mut(|i| {
            keys.down = i.consume_key(Modifiers::NONE, egui::Key::ArrowDown);
            keys.up = i.consume_key(Modifiers::NONE, egui::Key::ArrowUp);
            keys.accept = i.consume_key(Modifiers::NONE, egui::Key::Enter)
                || i.consume_key(Modifiers::NONE, egui::Key::Tab);
            keys.dismiss = i.consume_key(Modifiers::NONE, egui::Key::Escape);
        });
    }
    keys
}

/// Styled-source colours: Obsidian-like raw Markdown (headings large,
/// links/tags accented, code monospace) in the reading theme's colours.
pub(super) fn source_style(c: &crate::reading_theme::ThemeColors) -> HighlightStyle {
    HighlightStyle {
        base_size: 15.5,
        text: c.text.into(),
        dim: c.muted.into(),
        faint: egui::Color32::from(c.muted).gamma_multiply(0.75),
        accent: c.link.into(),
        code_bg: c.code_bg.into(),
        highlight_bg: c.highlight_bg.into(),
        semibold: egui::FontFamily::Name(theme::SEMIBOLD_FAMILY.into()),
        line_height: 24.0,
    }
}

/// The Markdown source editor plus its `/` and `[[` autocomplete popup.
#[allow(clippy::too_many_arguments)]
pub(super) fn source_editor(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    tr: &LocaleManager,
    editor: &mut MarkdownEditor,
    state: &mut EditorUi,
    vault: Option<&Vault>,
    pdfs: &[PathBuf],
    colors: &crate::reading_theme::ThemeColors,
    width: f32,
    rows: usize,
) {
    let p = pal();
    let edit_id = Id::new("note_body_editor");
    let keys = take_popup_keys(ctx, state);

    let mut body = editor.note.body.clone();
    let style = source_style(colors);
    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
        let job = crate::markdown::highlight::layout_job(text.as_str(), wrap_width, &style);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let output = egui::TextEdit::multiline(&mut body)
        .id(edit_id)
        .frame(Frame::NONE)
        .font(FontId::proportional(15.5))
        .text_color(colors.text.into())
        .hint_text(RichText::new(tr.t("editor-placeholder", &[])).color(p.text_faint))
        .desired_width(width)
        .desired_rows(rows)
        .lock_focus(true)
        .margin(Margin::ZERO)
        .layouter(&mut layouter)
        .show(ui);
    if body != editor.note.body {
        editor.set_body(body.clone());
    }

    if let Some((new_body, new_cursor)) = autocomplete(ctx, tr, state, vault, pdfs, &body, &output, &keys) {
        editor.set_body(new_body);
        set_cursor(ctx, edit_id, new_cursor);
    }
}

/// Moves the cursor of the `TextEdit` `id` to char index `cursor` and
/// focuses it.
pub(super) fn set_cursor(ctx: &egui::Context, id: Id, cursor: usize) {
    let mut edit_state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    edit_state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
    edit_state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Shows the `/` command or `[[` link popup for the focused `TextEdit`
/// whose text is `text`. When an item is chosen, returns the new text and
/// the cursor position (in chars) after the insertion. Updates
/// `state.popup_visible` either way.
#[allow(clippy::too_many_arguments)]
pub(super) fn autocomplete(
    ctx: &egui::Context,
    tr: &LocaleManager,
    state: &mut EditorUi,
    vault: Option<&Vault>,
    pdfs: &[PathBuf],
    text: &str,
    output: &egui::text_edit::TextEditOutput,
    keys: &PopupKeys,
) -> Option<(String, usize)> {
    let p = pal();
    let mut result = None;
    let mut visible = false;
    if let Some(range) = output.cursor_range
        && output.response.has_focus()
    {
        let cursor = range.primary;
        let char_idx = cursor.index.0;
        let byte = char_index_to_byte_offset(text, char_idx);
        let before = &text[..byte];

        let mut items: Vec<(String, Completion)> = if slash_menu_triggered(before) {
            slash_templates()
                .iter()
                .map(|tpl| (tr.t(tpl.key, &[]), Completion::Slash(tpl.insert)))
                .collect()
        } else if let Some(query) = wikilink_autocomplete_query(before) {
            vault
                .map(|v| WikilinkIndex::build(&v.notes).with_files(pdfs).suggestions(&query, 8))
                .unwrap_or_default()
                .into_iter()
                .map(|title| (title.clone(), Completion::Wikilink { query: query.clone(), title }))
                .collect()
        } else {
            Vec::new()
        };

        match state.popup_dismissed_at {
            Some(at) if at == byte => items.clear(),
            Some(_) => state.popup_dismissed_at = None,
            None => {}
        }
        if keys.dismiss && !items.is_empty() {
            state.popup_dismissed_at = Some(byte);
            items.clear();
        }

        if !items.is_empty() {
            visible = true;
            if !state.popup_visible {
                state.popup_index = 0;
            }
            state.popup_index = state.popup_index.min(items.len() - 1);
            if keys.down {
                state.popup_index = (state.popup_index + 1) % items.len();
            }
            if keys.up {
                state.popup_index = (state.popup_index + items.len() - 1) % items.len();
            }

            let cursor_rect = output.galley.pos_from_cursor(cursor).translate(output.galley_pos.to_vec2());
            let mut chosen = keys.accept.then_some(state.popup_index);
            let selected_index = state.popup_index;
            egui::Area::new(Id::new("editor_autocomplete_popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(cursor_rect.left_bottom() + Vec2::new(-6.0, 6.0))
                .show(ctx, |ui| {
                    theme::popover_frame().inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_min_width(240.0);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let header = match items[0].1 {
                            Completion::Slash(_) => tr.t("editor-slash-header", &[]),
                            Completion::Wikilink { .. } => tr.t("editor-link-header", &[]),
                        };
                        ui.label(RichText::new(header).size(theme::TEXT_XS).color(p.text_faint));
                        for (i, (label, completion)) in items.iter().enumerate() {
                            let icon = match completion {
                                Completion::Slash(_) => "/",
                                Completion::Wikilink { .. } => ICON_LINK.codepoint,
                            };
                            let resp = widgets::list_row(
                                ui,
                                widgets::RowSpec {
                                    icon,
                                    icon_color: p.text_faint,
                                    label,
                                    trailing: None,
                                    selected: i == selected_index,
                                    indent: 0.0,
                                    reserve_right: 0.0,
                                },
                            );
                            if resp.clicked() {
                                chosen = Some(i);
                            }
                        }
                        ui.label(RichText::new(tr.t("editor-popup-hint", &[])).size(theme::TEXT_XS).color(p.text_faint));
                    });
                });

            if let Some(i) = chosen {
                result = Some(match &items[i].1 {
                    Completion::Slash(insert) => {
                        let mut s = text.to_string();
                        s.replace_range(byte - 1..byte, insert);
                        (s, char_idx - 1 + insert.chars().count())
                    }
                    Completion::Wikilink { query, title } => {
                        let mut s = text.to_string();
                        let replacement = format!("{title}]]");
                        s.replace_range(byte - query.len()..byte, &replacement);
                        (s, char_idx - query.chars().count() + replacement.chars().count())
                    }
                });
                visible = false;
            }
        }
    }
    state.popup_visible = visible;
    result
}
