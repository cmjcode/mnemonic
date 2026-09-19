//! The Live line editor (§3.2.1): a note is always shown rendered, and the
//! block the user clicks — one line, or a whole table/fence/callout — turns
//! into raw Markdown right where it was. Everything else stays rendered.
//!
//! Keys behave like Obsidian's Live Preview: Enter splits the line (and
//! continues lists), Backspace at the start / Delete at the end joins with
//! the neighbouring line, ↑/↓ at the edge move to the neighbouring block,
//! Esc or a click elsewhere goes back to reading. Every edit goes through
//! `MarkdownEditor::set_body` (undo, autosave), splicing only the edited
//! lines with `live_blocks::replace_lines`.
//! Callers: `app::editor::show_editor` (Live and Split modes).

use std::ops::Range;
use std::path::PathBuf;

use egui::{Frame, Id, Margin, Modifiers, Pos2, RichText};
use egui_commonmark::CommonMarkCache;

use super::EditorUi;
use super::source::{autocomplete, set_cursor, source_style, take_popup_keys};
use crate::i18n::LocaleManager;
use crate::markdown::MarkdownEditor;
use crate::markdown::live_blocks::{self, ListContinuation, LiveBlock};
use crate::markdown::renderer::{EmbedResolver, LiveParams};
use crate::notes::Vault;
use crate::reading_theme::ThemeColors;

/// The one raw-Markdown `TextEdit` of the Live view.
pub(super) fn edit_id() -> Id {
    Id::new("live_line_editor")
}

/// The lines being edited as raw Markdown.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct LiveEdit {
    /// Body lines shown raw. An empty range at the end of the body is a
    /// new line that exists only once something is typed.
    pub(super) lines: Range<usize>,
    /// Where to put the text cursor once the editor is on screen.
    place: Option<Placement>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Placement {
    End,
    /// Char index into the edited text.
    Char(usize),
    /// Screen position of the click that opened the block.
    Point(Pos2),
}

/// Keys the Live editor handles itself (taken before the `TextEdit`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum NavKey {
    Leave,
    Enter,
    Up,
    Down,
    MergeUp,
    MergeDown,
}

/// Everything the Live view needs from the app besides the editor.
pub(super) struct LiveInputs<'a> {
    pub tr: &'a LocaleManager,
    pub vault: Option<&'a Vault>,
    pub pdfs: &'a [PathBuf],
    pub viewport: egui::Rect,
    pub colors: &'a ThemeColors,
    pub is_resolved: &'a dyn Fn(&str) -> bool,
    pub resolve_embed: &'a EmbedResolver<'a>,
}

/// What the Live view asks the app to do besides editing.
#[derive(Default)]
pub(super) struct LiveOutcome {
    pub navigate: Option<String>,
    pub tag: Option<String>,
}

/// Draws the note in Live mode and applies this frame's edits.
pub(super) fn live_view(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    editor: &mut MarkdownEditor,
    state: &mut EditorUi,
    inputs: &LiveInputs<'_>,
) -> LiveOutcome {
    let ctx = ui.ctx().clone();
    let id = edit_id();
    clamp_active(&mut state.live, &editor.note.body);

    let blocks = if state.live.is_some() { editor.blocks() } else { Vec::new() };
    let line_mode = state.live.as_ref().is_some_and(|l| is_line_mode(&blocks, &l.lines));
    let original = state.live.as_ref().map(|l| live_blocks::lines_text(&editor.note.body, l.lines.clone()));
    let rendered_range = state.live.as_ref().map(|l| l.lines.clone());

    let popup_keys = take_popup_keys(&ctx, state);
    let nav = match &original {
        Some(text) if !state.popup_visible && ctx.memory(|m| m.has_focus(id)) => take_nav_key(&ctx, id, text, line_mode),
        _ => None,
    };

    // ── Render, with the raw editor in place of the active lines ──
    let mut buffer = original.clone().unwrap_or_default();
    let mut output: Option<egui::text_edit::TextEditOutput> = None;
    let style = source_style(inputs.colors);
    let hint = inputs.tr.t("editor-live-hint", &[]);
    let edit_fill = egui::Color32::from(inputs.colors.code_bg).gamma_multiply(0.55);
    let mut draw_editor = |ui: &mut egui::Ui| {
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let job = crate::markdown::highlight::layout_job(text.as_str(), wrap_width, &style);
            ui.fonts_mut(|f| f.layout_job(job))
        };
        Frame::NONE
            .fill(edit_fill)
            .corner_radius(6)
            .inner_margin(Margin::symmetric(8, 3))
            .outer_margin(Margin { left: -8, right: -8, top: 0, bottom: 0 })
            .show(ui, |ui| {
                output = Some(
                    egui::TextEdit::multiline(&mut buffer)
                        .id(id)
                        .frame(Frame::NONE)
                        .desired_width(ui.available_width())
                        .desired_rows(1)
                        .lock_focus(true)
                        .margin(Margin::ZERO)
                        .hint_text(RichText::new(&hint).color(egui::Color32::from(inputs.colors.muted)))
                        .layouter(&mut layouter)
                        .show(ui),
                );
            });
    };
    let placeholder = inputs.tr.t("editor-placeholder", &[]);
    let params = LiveParams {
        viewport: inputs.viewport,
        is_resolved: inputs.is_resolved,
        resolve_embed: inputs.resolve_embed,
        colors: inputs.colors,
        active: rendered_range.clone(),
        placeholder: &placeholder,
    };
    let outcome = editor.render(ui, cache, &params, &mut draw_editor);

    let result = LiveOutcome { navigate: outcome.clicked_wikilink, tag: outcome.clicked_tag };
    if let Some(body) = outcome.updated_body {
        editor.set_body(body);
    }

    // ── Apply this frame's edits to the active lines ──
    let mut live = state.live.take();
    if let (Some(l), Some(orig)) = (live.as_mut(), original.as_deref()) {
        let cursor = output
            .as_ref()
            .and_then(|o| o.cursor_range)
            .map(|r| r.primary.index.0)
            .unwrap_or_else(|| orig.chars().count());
        match nav {
            Some(NavKey::Leave) => {}
            Some(key) => handle_key(editor, l, &blocks, orig, cursor, key),
            None if buffer != orig => commit(editor, l, &buffer, cursor, line_mode, false),
            None => {}
        }
    }
    if nav == Some(NavKey::Leave) {
        ctx.memory_mut(|m| m.surrender_focus(id));
        live = None;
    }
    if let (Some(l), Some(out)) = (live.as_mut(), output.as_ref())
        && let Some((text, cursor)) =
            autocomplete(&ctx, inputs.tr, state, inputs.vault, inputs.pdfs, &buffer, out, &popup_keys)
    {
        commit(editor, l, &text, cursor, line_mode, true);
    }

    // Place the cursor once the editor for these lines is on screen.
    if let (Some(l), Some(out)) = (live.as_mut(), output.as_ref())
        && Some(&l.lines) == rendered_range.as_ref()
        && let Some(place) = l.place.take()
    {
        let len = buffer.chars().count();
        let at = match place {
            Placement::End => len,
            Placement::Char(c) => c.min(len),
            Placement::Point(p) => out.galley.cursor_from_pos(p - out.galley_pos).index.0.min(len),
        };
        set_cursor(&ctx, id, at);
    }

    // A line that became part of a table or a closed fence pulls the
    // whole block into the editor.
    if let (Some(l), Some(lines)) = (live.as_mut(), outcome.active_lines)
        && Some(&l.lines) == rendered_range.as_ref()
        && l.lines != lines
    {
        l.lines = lines;
        l.place.get_or_insert(Placement::End);
        ctx.request_repaint();
    }

    // Leaving: focus went elsewhere (unless a popup item was being clicked).
    if let Some(out) = output.as_ref()
        && out.response.lost_focus()
        && !state.popup_visible
        && nav.is_none()
    {
        live = None;
    }

    // ── Clicks that start editing somewhere else ──
    let mut open = |lines: Range<usize>, place: Placement| {
        live = Some(LiveEdit { lines, place: Some(place) });
        ctx.request_repaint();
    };
    if let Some(req) = outcome.edit {
        open(req.lines, req.pos.map_or(Placement::End, Placement::Point));
    } else if outcome.clicked_after_end {
        let body = &editor.note.body;
        let total = live_blocks::line_count(body);
        let last_blank = body.lines().last().is_some_and(|l| l.trim().is_empty());
        open(if last_blank { total - 1..total } else { total..total }, Placement::End);
    } else if let Some(line) = outcome.clicked_source_line
        && let Some(block) = editor.blocks().into_iter().find(|b| b.lines.contains(&line))
    {
        // A Mermaid node without a link: edit the diagram at its line.
        let text = live_blocks::lines_text(&editor.note.body, block.lines.start..line);
        let offset = if line > block.lines.start { text.chars().count() + 1 } else { 0 };
        open(block.lines, Placement::Char(offset));
    }
    state.live = live;
    result
}

/// Keeps the active range inside the body (after undo, reload, …).
fn clamp_active(live: &mut Option<LiveEdit>, body: &str) {
    let total = live_blocks::line_count(body);
    let Some(l) = live.as_mut() else {
        return;
    };
    if l.lines.start > total {
        *live = None;
        return;
    }
    l.lines.end = l.lines.end.clamp(l.lines.start, total);
    if l.lines.is_empty() && l.lines.start < total {
        l.lines.end = l.lines.start + 1;
    }
}

/// Single-line editing (Enter splits, Backspace joins) unless the lines
/// are an atomic block such as a table or a code fence.
fn is_line_mode(blocks: &[LiveBlock], lines: &Range<usize>) -> bool {
    lines.len() <= 1
        && !blocks
            .iter()
            .any(|b| b.kind.is_atomic() && b.lines.start <= lines.start && lines.start < b.lines.end)
}

fn take_nav_key(ctx: &egui::Context, id: Id, text: &str, line_mode: bool) -> Option<NavKey> {
    let range = egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range());
    let len = text.chars().count();
    let (cursor, collapsed) = match range {
        Some(r) => (r.primary.index.0.min(len), r.primary.index == r.secondary.index),
        None => (len, true),
    };
    let on_first_row = !text.chars().take(cursor).any(|c| c == '\n');
    let on_last_row = !text.chars().skip(cursor).any(|c| c == '\n');
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, egui::Key::Escape) {
            return Some(NavKey::Leave);
        }
        if !collapsed {
            return None;
        }
        let mut key = |key| i.consume_key(Modifiers::NONE, key);
        if line_mode && key(egui::Key::Enter) {
            Some(NavKey::Enter)
        } else if line_mode && cursor == 0 && key(egui::Key::Backspace) {
            Some(NavKey::MergeUp)
        } else if line_mode && cursor == len && key(egui::Key::Delete) {
            Some(NavKey::MergeDown)
        } else if on_first_row && key(egui::Key::ArrowUp) {
            Some(NavKey::Up)
        } else if on_last_row && key(egui::Key::ArrowDown) {
            Some(NavKey::Down)
        } else {
            None
        }
    })
}

/// Writes `text` over the active lines. In line mode a text that now holds
/// line breaks (Enter, paste, a multi-line slash template) stays in the
/// body as several lines, and editing follows the cursor to its line.
/// `external` = the text didn't come from typing, so the `TextEdit`
/// cursor must be moved to `cursor` explicitly.
fn commit(editor: &mut MarkdownEditor, live: &mut LiveEdit, text: &str, cursor: usize, line_mode: bool, external: bool) {
    let start = live.lines.start;
    editor.set_body(live_blocks::replace_lines(&editor.note.body, live.lines.clone(), text));
    if line_mode && text.contains('\n') {
        let before: String = text.chars().take(cursor).collect();
        let row = before.matches('\n').count();
        let col = before.rsplit('\n').next().unwrap_or("").chars().count();
        live.lines = start + row..start + row + 1;
        live.place = Some(Placement::Char(col));
    } else {
        live.lines = start..start + text.split('\n').count();
        if external {
            live.place = Some(Placement::Char(cursor));
        }
    }
}

fn handle_key(editor: &mut MarkdownEditor, live: &mut LiveEdit, blocks: &[LiveBlock], text: &str, cursor: usize, key: NavKey) {
    let prev = blocks.iter().find(|b| b.lines.end == live.lines.start);
    let next = blocks.iter().find(|b| b.lines.start == live.lines.end && !live.lines.is_empty());
    let single_line = |b: &LiveBlock| b.lines.len() == 1 && !b.kind.is_atomic();
    let before: String = text.chars().take(cursor).collect();
    let column = before.rsplit('\n').next().unwrap_or("").chars().count();
    match key {
        NavKey::Leave => {}
        NavKey::Enter => {
            let byte = crate::markdown::editor::char_index_to_byte_offset(text, cursor);
            let (left, right) = text.split_at(byte);
            match live_blocks::continue_list(text) {
                // Enter on an empty item ends the list: clear the marker.
                ListContinuation::EndList if right.trim().is_empty() => commit(editor, live, "", 0, true, true),
                cont => {
                    let prefix = match cont {
                        ListContinuation::Continue(p) => p,
                        _ => String::new(),
                    };
                    let joined = format!("{left}\n{prefix}{}", right.trim_start());
                    let at = left.chars().count() + 1 + prefix.chars().count();
                    commit(editor, live, &joined, at, true, true);
                }
            }
        }
        NavKey::MergeUp => match prev {
            Some(p) if single_line(p) => {
                let above = live_blocks::lines_text(&editor.note.body, p.lines.clone());
                let at = above.chars().count();
                let merged = format!("{above}{text}");
                live.lines = p.lines.start..live.lines.end.max(p.lines.end);
                commit(editor, live, &merged, at, true, true);
            }
            Some(p) => {
                live.lines = p.lines.clone();
                live.place = Some(Placement::End);
            }
            None => {}
        },
        NavKey::MergeDown => {
            if let Some(n) = next.filter(|n| single_line(n)) {
                let below = live_blocks::lines_text(&editor.note.body, n.lines.clone());
                let merged = format!("{text}{below}");
                live.lines = live.lines.start..n.lines.end;
                commit(editor, live, &merged, cursor, true, true);
            }
        }
        NavKey::Up => {
            if let Some(p) = prev {
                live.lines = p.lines.clone();
                live.place = Some(if single_line(p) { Placement::Char(column) } else { Placement::End });
            }
        }
        NavKey::Down => {
            if let Some(n) = next {
                live.lines = n.lines.clone();
                live.place = Some(Placement::Char(column));
            } else if !live.lines.is_empty()
                && live.lines.end == live_blocks::line_count(&editor.note.body)
                && !text.trim().is_empty()
            {
                // ↓ on the last line starts a new one, like clicking below it.
                live.lines = live.lines.end..live.lines.end;
                live.place = Some(Placement::End);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::Note;

    fn editor(body: &str) -> (tempfile::TempDir, MarkdownEditor) {
        let dir = tempfile::tempdir().unwrap();
        let note = Note::create(dir.path(), "Uji", body).unwrap();
        (dir, MarkdownEditor::open(note))
    }

    fn live(lines: Range<usize>) -> LiveEdit {
        LiveEdit { lines, place: None }
    }

    #[test]
    fn typing_replaces_only_the_active_line() {
        let (_d, mut e) = editor("# Judul\nsatu\ndua\n");
        let mut l = live(1..2);
        commit(&mut e, &mut l, "satu!", 5, true, false);
        assert_eq!(e.note.body, "# Judul\nsatu!\ndua\n");
        assert_eq!(l.lines, 1..2);
        assert_eq!(l.place, None, "typed text keeps the TextEdit cursor");
    }

    #[test]
    fn enter_splits_and_continues_lists() {
        let (_d, mut e) = editor("- beli beras\nakhir");
        let blocks = e.blocks();
        let mut l = live(0..1);
        handle_key(&mut e, &mut l, &blocks, "- beli beras", 12, NavKey::Enter);
        assert_eq!(e.note.body, "- beli beras\n- \nakhir");
        assert_eq!(l.lines, 1..2);
        assert_eq!(l.place, Some(Placement::Char(2)));

        // Enter on the now-empty item ends the list.
        let blocks = e.blocks();
        handle_key(&mut e, &mut l, &blocks, "- ", 2, NavKey::Enter);
        assert_eq!(e.note.body, "- beli beras\n\nakhir");
        assert_eq!(l.lines, 1..2);
    }

    #[test]
    fn enter_in_the_middle_moves_the_rest_down() {
        let (_d, mut e) = editor("halo dunia");
        let blocks = e.blocks();
        let mut l = live(0..1);
        handle_key(&mut e, &mut l, &blocks, "halo dunia", 4, NavKey::Enter);
        assert_eq!(e.note.body, "halo\ndunia");
        assert_eq!((l.lines.clone(), l.place), (1..2, Some(Placement::Char(0))));
    }

    #[test]
    fn backspace_and_delete_join_lines() {
        let (_d, mut e) = editor("satu\ndua\ntiga");
        let blocks = e.blocks();
        let mut l = live(1..2);
        handle_key(&mut e, &mut l, &blocks, "dua", 0, NavKey::MergeUp);
        assert_eq!(e.note.body, "satudua\ntiga");
        assert_eq!((l.lines.clone(), l.place), (0..1, Some(Placement::Char(4))));

        let blocks = e.blocks();
        handle_key(&mut e, &mut l, &blocks, "satudua", 7, NavKey::MergeDown);
        assert_eq!(e.note.body, "satuduatiga");
        assert_eq!(l.lines, 0..1);
    }

    #[test]
    fn arrows_move_between_blocks_and_into_tables_whole() {
        let (_d, mut e) = editor("a\n| x |\n| --- |\n| 1 |\nb");
        let blocks = e.blocks();
        let mut l = live(4..5);
        handle_key(&mut e, &mut l, &blocks, "b", 1, NavKey::Up);
        assert_eq!(l.lines, 1..4);
        assert_eq!(l.place, Some(Placement::End));
        handle_key(&mut e, &mut l, &blocks, "| x |\n| --- |\n| 1 |", 0, NavKey::Up);
        assert_eq!(l.lines, 0..1);
        assert!(is_line_mode(&blocks, &(0..1)));
        assert!(!is_line_mode(&blocks, &(1..4)));
        assert!(!is_line_mode(&blocks, &(2..3)), "a line inside a table edits the table");
    }

    #[test]
    fn new_line_at_the_end_exists_once_typed() {
        let (_d, mut e) = editor("isi\n");
        let mut l = live(1..1);
        commit(&mut e, &mut l, "baru", 4, true, false);
        assert_eq!(e.note.body, "isi\nbaru\n");
        assert_eq!(l.lines, 1..2);

        let mut gone = Some(live(9..10));
        clamp_active(&mut gone, "a");
        assert!(gone.is_none());
        let mut shrunk = Some(live(0..5));
        clamp_active(&mut shrunk, "a\nb");
        assert_eq!(shrunk.unwrap().lines, 0..2);
        let mut virtual_end = Some(live(2..2));
        clamp_active(&mut virtual_end, "a\nb");
        assert_eq!(virtual_end.unwrap().lines, 2..2);
    }

    #[test]
    fn pasted_lines_split_and_follow_the_cursor() {
        let (_d, mut e) = editor("x\ny");
        let mut l = live(0..1);
        commit(&mut e, &mut l, "x1\nx2\nx3", 8, true, false);
        assert_eq!(e.note.body, "x1\nx2\nx3\ny");
        assert_eq!((l.lines.clone(), l.place), (2..3, Some(Placement::Char(2))));
    }
}
