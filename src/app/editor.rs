//! Note editor screen: a centered, comfortable writing column (Write /
//! Read modes), an optional outline + backlinks panel, an inline
//! autocomplete popup for `/` commands and `[[wikilinks]]`, and the
//! infinite canvas for Canvas mode.

use egui::{Align2, FontId, Frame, Id, Margin, Modifiers, RichText, Vec2};
use egui_icons::icons::{ICON_DESCRIPTION, ICON_LINK};

use super::MnemonicApp;
use crate::canvas::{
    self, CanvasDocument, CanvasElement, CanvasElementId, CanvasTool, InteractionState,
};
use crate::i18n::LocaleManager;
use crate::markdown::editor::{
    char_index_to_byte_offset, slash_menu_triggered, slash_templates, wikilink_autocomplete_query,
};
use crate::markdown::{EditorMode, MarkdownEditor, WikilinkIndex, wikilink};
use crate::notes::Vault;
use crate::ui::{self, ToastKind, pal, theme, widgets};

/// Per-open-note UI state that isn't part of the document itself.
#[derive(Default)]
pub(super) struct EditorUi {
    /// Working copy of the title shown in the top bar.
    pub(super) title_buffer: String,
    pub(super) save_failed: bool,
    /// Focus the title with its text selected on the next frame.
    pub(super) select_title: bool,
    /// Whether the autocomplete popup was showing last frame.
    pub(super) popup_visible: bool,
    popup_index: usize,
    /// Byte offset of a trigger the user dismissed with Esc, so the popup
    /// stays closed until the cursor moves.
    popup_dismissed_at: Option<usize>,
}

impl EditorUi {
    pub(super) fn for_title(title: &str) -> Self {
        EditorUi {
            title_buffer: title.to_string(),
            ..Default::default()
        }
    }
}

enum Completion {
    Slash(&'static str),
    Wikilink { query: String, title: String },
}

/// What the canvas surface asks the app to do after rendering.
pub(super) struct CanvasOutcome {
    pub(super) modified: bool,
    pub(super) toast: Option<(ToastKind, String)>,
}

impl MnemonicApp {
    pub(super) fn show_editor(&mut self, ui: &mut egui::Ui) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let tr = &self.locales;
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let ctx = ui.ctx().clone();

        let mut navigate_to: Option<String> = None;
        let mut scroll_to_slug: Option<String> = None;
        let mut canvas_toast = None;

        if editor.mode == EditorMode::Edgeless {
            editor.ensure_canvas();
            if let Some(canvas) = editor.canvas.as_mut() {
                let outcome =
                    show_canvas_surface(canvas, &mut editor.canvas_interaction, ui, p.is_dark, tr);
                if outcome.modified {
                    editor.sync_canvas_to_body();
                }
                canvas_toast = outcome.toast;
            }
            self.editor_ui.popup_visible = false;
        } else {
            // ── Outline & backlinks panel ──
            if self.settings.show_outline {
                let outline = editor.outline();
                let backlinks: Vec<String> = self
                    .vault
                    .as_ref()
                    .map(|v| {
                        wikilink::backlinks_for(
                            &editor.note.frontmatter.title,
                            editor.note.frontmatter.id,
                            &v.notes,
                        )
                        .into_iter()
                        .map(|n| n.frontmatter.title.clone())
                        .collect()
                    })
                    .unwrap_or_default();

                egui::Panel::right("editor_outline_panel")
                    .resizable(true)
                    .default_size(240.0)
                    .size_range(180.0..=380.0)
                    .frame(theme::side_panel_frame().fill(p.bg))
                    .show_separator_line(true)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            widgets::section_header(ui, &t("editor-outline"));
                            if outline.is_empty() {
                                hint_text(ui, &t("editor-outline-empty"));
                            }
                            for heading in &outline {
                                let resp = widgets::list_row(
                                    ui,
                                    widgets::RowSpec {
                                        icon: "",
                                        icon_color: p.text_faint,
                                        label: &heading.title,
                                        trailing: None,
                                        selected: false,
                                        indent: (heading.level.saturating_sub(1) as f32) * 12.0,
                                        reserve_right: 0.0,
                                    },
                                );
                                if resp.clicked() {
                                    scroll_to_slug = Some(heading.slug.clone());
                                }
                            }

                            widgets::section_header(ui, &t("editor-backlinks"));
                            if backlinks.is_empty() {
                                hint_text(ui, &t("editor-backlinks-empty"));
                            }
                            for title in &backlinks {
                                let resp = widgets::list_row(
                                    ui,
                                    widgets::RowSpec {
                                        icon: ICON_DESCRIPTION.codepoint,
                                        icon_color: p.note_icon,
                                        label: title,
                                        trailing: None,
                                        selected: false,
                                        indent: 0.0,
                                        reserve_right: 0.0,
                                    },
                                );
                                if resp.clicked() {
                                    navigate_to = Some(title.clone());
                                }
                            }
                        });
                    });
            }

            // ── Writing column ──
            let stats = format!(
                "{} · {}",
                tr.t(
                    "editor-word-count",
                    &[("count", &editor.word_count().to_string())]
                ),
                tr.t(
                    "editor-reading-time",
                    &[("minutes", &editor.reading_time_minutes().to_string())]
                ),
            );
            let editor_ui = &mut self.editor_ui;
            let cache = &mut self.markdown_cache;
            let vault = self.vault.as_ref();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_viewport(ui, |ui, viewport| {
                    let content_top = ui.min_rect().top();
                    let avail = ui.available_width();
                    let col = (avail - 2.0 * theme::SPACE_XL).clamp(200.0, theme::EDITOR_MAX_WIDTH);
                    let margin = ((avail - col) / 2.0).max(0.0);
                    ui.add_space(theme::SPACE_XL);
                    ui.horizontal_top(|ui| {
                        ui.add_space(margin);
                        ui.vertical(|ui| {
                            ui.set_width(col);
                            ui.label(
                                RichText::new(&stats)
                                    .size(theme::TEXT_XS)
                                    .color(p.text_faint),
                            );
                            ui.add_space(theme::SPACE_M);

                            match editor.mode {
                                EditorMode::Source => {
                                    let rows =
                                        ((viewport.height() - 120.0) / 22.0).max(12.0) as usize;
                                    source_editor(
                                        ui,
                                        &ctx,
                                        tr,
                                        &mut editor,
                                        editor_ui,
                                        vault,
                                        col,
                                        rows,
                                    );
                                }
                                EditorMode::Reading => {
                                    editor_ui.popup_visible = false;
                                    // The renderer virtualizes against heights
                                    // measured from its own first line, so shift
                                    // the viewport past the padding above it.
                                    let offset = ui.cursor().top() - content_top;
                                    let local = viewport.translate(Vec2::new(0.0, -offset));
                                    let outcome = editor.render(ui, cache, local);
                                    if let Some(new_body) = outcome.updated_body {
                                        editor.set_body(new_body);
                                    }
                                    if let Some(title) = outcome.clicked_wikilink {
                                        navigate_to = Some(title);
                                    }
                                }
                                EditorMode::Edgeless => {}
                            }
                            ui.add_space(theme::SPACE_XL * 4.0);
                        });
                    });
                });
        }

        if let Some(slug) = scroll_to_slug {
            // The outline scrolls the rendered view, so jump to Read mode.
            editor.mode = EditorMode::Reading;
            self.markdown_cache.scroll_to_id_target_mut().replace(slug);
        }
        self.editor = Some(editor);
        if let Some((kind, msg)) = canvas_toast {
            self.toasts.push(kind, msg);
        }
        if let Some(title) = navigate_to {
            self.navigate_wikilink(&title);
        }
    }
}

fn hint_text(ui: &mut egui::Ui, text: &str) {
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(theme::TEXT_XS)
                    .color(pal().text_faint),
            );
        });
}

/// The Markdown source editor plus its `/` and `[[` autocomplete popup.
#[allow(clippy::too_many_arguments)]
fn source_editor(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    tr: &LocaleManager,
    editor: &mut MarkdownEditor,
    state: &mut EditorUi,
    vault: Option<&Vault>,
    width: f32,
    rows: usize,
) {
    let p = pal();
    let edit_id = Id::new("note_body_editor");

    // Popup navigation keys must be consumed before the TextEdit sees them.
    let (mut down, mut up, mut accept, mut dismiss) = (false, false, false, false);
    if state.popup_visible {
        ctx.input_mut(|i| {
            down = i.consume_key(Modifiers::NONE, egui::Key::ArrowDown);
            up = i.consume_key(Modifiers::NONE, egui::Key::ArrowUp);
            accept = i.consume_key(Modifiers::NONE, egui::Key::Enter)
                || i.consume_key(Modifiers::NONE, egui::Key::Tab);
            dismiss = i.consume_key(Modifiers::NONE, egui::Key::Escape);
        });
    }

    let mut body = editor.note.body.clone();
    let output = egui::TextEdit::multiline(&mut body)
        .id(edit_id)
        .frame(Frame::NONE)
        .font(FontId::proportional(15.5))
        .text_color(p.text)
        .hint_text(RichText::new(tr.t("editor-placeholder", &[])).color(p.text_faint))
        .desired_width(width)
        .desired_rows(rows)
        .lock_focus(true)
        .margin(Margin::ZERO)
        .show(ui);
    if body != editor.note.body {
        editor.set_body(body.clone());
    }

    let mut visible = false;
    if let Some(range) = output.cursor_range
        && output.response.has_focus()
    {
        let cursor = range.primary;
        let char_idx = cursor.index.0;
        let byte = char_index_to_byte_offset(&body, char_idx);
        let before = &body[..byte];

        let mut items: Vec<(String, Completion)> = if slash_menu_triggered(before) {
            slash_templates()
                .iter()
                .map(|tpl| (tr.t(tpl.key, &[]), Completion::Slash(tpl.insert)))
                .collect()
        } else if let Some(query) = wikilink_autocomplete_query(before) {
            vault
                .map(|v| WikilinkIndex::build(&v.notes).suggestions(&query, 8))
                .unwrap_or_default()
                .into_iter()
                .map(|title| {
                    (
                        title.clone(),
                        Completion::Wikilink {
                            query: query.clone(),
                            title,
                        },
                    )
                })
                .collect()
        } else {
            Vec::new()
        };

        match state.popup_dismissed_at {
            Some(at) if at == byte => items.clear(),
            Some(_) => state.popup_dismissed_at = None,
            None => {}
        }
        if dismiss && !items.is_empty() {
            state.popup_dismissed_at = Some(byte);
            items.clear();
        }

        if !items.is_empty() {
            visible = true;
            if !state.popup_visible {
                state.popup_index = 0;
            }
            state.popup_index = state.popup_index.min(items.len() - 1);
            if down {
                state.popup_index = (state.popup_index + 1) % items.len();
            }
            if up {
                state.popup_index = (state.popup_index + items.len() - 1) % items.len();
            }

            let cursor_rect = output
                .galley
                .pos_from_cursor(cursor)
                .translate(output.galley_pos.to_vec2());
            let mut chosen = accept.then_some(state.popup_index);
            let selected_index = state.popup_index;
            egui::Area::new(Id::new("editor_autocomplete_popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(cursor_rect.left_bottom() + Vec2::new(-6.0, 6.0))
                .show(ctx, |ui| {
                    theme::popover_frame()
                        .inner_margin(Margin::same(6))
                        .show(ui, |ui| {
                            ui.set_min_width(240.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let header = match items[0].1 {
                                Completion::Slash(_) => tr.t("editor-slash-header", &[]),
                                Completion::Wikilink { .. } => tr.t("editor-link-header", &[]),
                            };
                            ui.label(
                                RichText::new(header)
                                    .size(theme::TEXT_XS)
                                    .color(p.text_faint),
                            );
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
                            ui.label(
                                RichText::new(tr.t("editor-popup-hint", &[]))
                                    .size(theme::TEXT_XS)
                                    .color(p.text_faint),
                            );
                        });
                });

            if let Some(i) = chosen {
                let (new_body, new_cursor) = match &items[i].1 {
                    Completion::Slash(insert) => {
                        let mut s = body.clone();
                        s.replace_range(byte - 1..byte, insert);
                        (s, char_idx - 1 + insert.chars().count())
                    }
                    Completion::Wikilink { query, title } => {
                        let mut s = body.clone();
                        let replacement = format!("{title}]]");
                        s.replace_range(byte - query.len()..byte, &replacement);
                        (
                            s,
                            char_idx - query.chars().count() + replacement.chars().count(),
                        )
                    }
                };
                editor.set_body(new_body);
                if let Some(mut edit_state) = egui::TextEdit::load_state(ctx, edit_id) {
                    edit_state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(
                            egui::text::CCursor::new(new_cursor),
                        )));
                    edit_state.store(ctx, edit_id);
                }
                ctx.memory_mut(|m| m.request_focus(edit_id));
                visible = false;
            }
        }
    }
    state.popup_visible = visible;
}

/// Renders the interactive infinite canvas (Canvas mode). Tool docks and
/// HUDs are positioned inside the canvas rect so they never overlap the
/// sidebar or other panels.
pub(super) fn show_canvas_surface(
    canvas: &mut CanvasDocument,
    interaction: &mut InteractionState,
    ui: &mut egui::Ui,
    is_dark: bool,
    tr: &LocaleManager,
) -> CanvasOutcome {
    let (response, painter) = ui.allocate_painter(
        ui.available_size_before_wrap(),
        egui::Sense::click_and_drag(),
    );
    let screen_rect = response.rect;
    let origin = screen_rect.min;
    let mut modified = false;
    let mut toast = None;
    let ctx = ui.ctx().clone();

    // Single-letter tool shortcuts (V, H, S, R, ...) while nothing is focused.
    if !ctx.egui_wants_keyboard_input()
        && interaction.editing_text_elem.is_none()
        && let Some(tool) = ui::left_toolbar::tool_shortcut_pressed(&ctx)
    {
        interaction.active_tool = tool;
    }

    // 1. Zoom & pan — only while the pointer is over the canvas.
    if response.contains_pointer() {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        let zoom_delta = ui.input(|i| i.zoom_delta());
        let ctrl_pressed = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let hover_pos = response.hover_pos();

        if (zoom_delta - 1.0).abs() > 1e-4 {
            if let Some(pos) = hover_pos {
                canvas.viewport.zoom_at(zoom_delta, pos, origin);
                modified = true;
            }
        } else if ctrl_pressed && scroll_delta.y != 0.0 {
            let factor = if scroll_delta.y > 0.0 { 1.1 } else { 0.9 };
            if let Some(pos) = hover_pos {
                canvas.viewport.zoom_at(factor, pos, origin);
                modified = true;
            }
        } else if scroll_delta != Vec2::ZERO {
            canvas
                .viewport
                .add_pan_vec(scroll_delta / canvas.viewport.zoom);
            modified = true;
        }
    }

    // 2. Dot grid.
    canvas.viewport.draw_grid(&painter, screen_rect, is_dark);

    // 3. Tool drags.
    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let world_pos = canvas.viewport.screen_to_world(pos, origin);
            interaction.is_dragging = true;
            interaction.drag_start_world = Some([world_pos.x, world_pos.y]);
            interaction.drag_current_world = Some([world_pos.x, world_pos.y]);
            if interaction.active_tool == CanvasTool::Pen {
                interaction.current_freehand_points = vec![[world_pos.x, world_pos.y]];
            }
        }
    } else if response.dragged() {
        let drag_delta = response.drag_delta();
        if let Some(pos) = response.interact_pointer_pos() {
            let world_pos = canvas.viewport.screen_to_world(pos, origin);
            interaction.drag_current_world = Some([world_pos.x, world_pos.y]);
            match interaction.active_tool {
                CanvasTool::Pan => {
                    canvas
                        .viewport
                        .add_pan_vec(drag_delta / canvas.viewport.zoom);
                    modified = true;
                }
                CanvasTool::Pen => {
                    interaction
                        .current_freehand_points
                        .push([world_pos.x, world_pos.y]);
                }
                CanvasTool::Select => {
                    if let Some(start) = interaction.drag_start_world {
                        let zoom = canvas.viewport.zoom;
                        let start_world = egui::Pos2::new(start[0], start[1]);
                        match canvas.element_at(start_world).map(|e| e.id()) {
                            Some(elem_id) => {
                                if let Some(target) = canvas.get_element_mut(elem_id) {
                                    target.translate(drag_delta / zoom);
                                    // Track the grab point so the element
                                    // stays "held" as it moves.
                                    interaction.drag_start_world = Some([
                                        start[0] + drag_delta.x / zoom,
                                        start[1] + drag_delta.y / zoom,
                                    ]);
                                    modified = true;
                                }
                            }
                            None => {
                                canvas.viewport.add_pan_vec(drag_delta / zoom);
                                modified = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    } else if response.drag_stopped() {
        if let (Some(start), Some(curr)) =
            (interaction.drag_start_world, interaction.drag_current_world)
        {
            let min_x = start[0].min(curr[0]);
            let min_y = start[1].min(curr[1]);
            let w = (start[0].max(curr[0]) - min_x).max(80.0);
            let h = (start[1].max(curr[1]) - min_y).max(50.0);

            match interaction.active_tool {
                CanvasTool::StickyNote => {
                    canvas.add_element(CanvasElement::StickyNote {
                        id: CanvasElementId::new(),
                        pos: [min_x, min_y],
                        size: [w.max(180.0), h.max(120.0)],
                        text: tr.t("canvas-new-sticky", &[]),
                        color: interaction.primary_color,
                    });
                    interaction.active_tool = CanvasTool::Select;
                    modified = true;
                }
                CanvasTool::Shape(kind) => {
                    canvas.add_element(CanvasElement::Shape {
                        id: CanvasElementId::new(),
                        kind,
                        rect: [min_x, min_y, min_x + w.max(140.0), min_y + h.max(80.0)],
                        stroke_color: interaction.primary_color,
                        stroke_width: interaction.stroke_width,
                        fill_color: None,
                        text: String::new(),
                        text_color: None,
                    });
                    interaction.active_tool = CanvasTool::Select;
                    modified = true;
                }
                CanvasTool::Connector => {
                    canvas.add_element(CanvasElement::Connector {
                        id: CanvasElementId::new(),
                        from_elem: None,
                        to_elem: None,
                        from_pos: start,
                        to_pos: curr,
                        routing: canvas::ConnectorRouting::Straight,
                        stroke_color: interaction.primary_color,
                        stroke_width: interaction.stroke_width,
                        label: String::new(),
                        arrow_end: true,
                        waypoints: Vec::new(),
                    });
                    interaction.active_tool = CanvasTool::Select;
                    modified = true;
                }
                CanvasTool::Pen => {
                    if interaction.current_freehand_points.len() >= 2 {
                        canvas.add_element(CanvasElement::FreehandStroke {
                            id: CanvasElementId::new(),
                            points: std::mem::take(&mut interaction.current_freehand_points),
                            color: interaction.primary_color,
                            width: interaction.stroke_width,
                        });
                        modified = true;
                    }
                }
                CanvasTool::Eraser => {
                    if let Some(id) = canvas
                        .element_at(egui::Pos2::new(start[0], start[1]))
                        .map(|e| e.id())
                    {
                        canvas.remove_element(id);
                        modified = true;
                    }
                }
                _ => {}
            }
        }
        interaction.is_dragging = false;
        interaction.drag_start_world = None;
        interaction.drag_current_world = None;
    }

    // The eraser also works with a simple click.
    if response.clicked()
        && interaction.active_tool == CanvasTool::Eraser
        && let Some(pos) = response.interact_pointer_pos()
    {
        let world = canvas.viewport.screen_to_world(pos, origin);
        if let Some(id) = canvas.element_at(world).map(|e| e.id()) {
            canvas.remove_element(id);
            modified = true;
        }
    }

    // 4. Elements.
    let hovered_id = response.hover_pos().and_then(|pos| {
        let world = canvas.viewport.screen_to_world(pos, origin);
        canvas.element_at(world).map(|e| e.id())
    });
    for elem in &canvas.elements {
        canvas::draw_element(
            &painter,
            &canvas.viewport,
            origin,
            elem,
            hovered_id == Some(elem.id()),
            is_dark,
        );
    }

    if interaction.active_tool == CanvasTool::Pen && interaction.current_freehand_points.len() >= 2
    {
        let c = interaction.primary_color;
        let stroke_c = egui::Color32::from_rgb(
            (c[0] * 255.0) as u8,
            (c[1] * 255.0) as u8,
            (c[2] * 255.0) as u8,
        );
        let points: Vec<egui::Pos2> = interaction
            .current_freehand_points
            .iter()
            .map(|pt| {
                canvas
                    .viewport
                    .world_to_screen(egui::Pos2::new(pt[0], pt[1]), origin)
            })
            .collect();
        for w in points.windows(2) {
            painter.line_segment(
                [w[0], w[1]],
                (interaction.stroke_width * canvas.viewport.zoom, stroke_c),
            );
        }
    }

    if response.hovered() {
        ctx.set_cursor_icon(match interaction.active_tool {
            CanvasTool::Pan => egui::CursorIcon::Grab,
            CanvasTool::Select if hovered_id.is_some() => egui::CursorIcon::Move,
            CanvasTool::Select => egui::CursorIcon::Default,
            _ => egui::CursorIcon::Crosshair,
        });
    }

    if canvas.elements.is_empty() {
        painter.text(
            screen_rect.center(),
            Align2::CENTER_CENTER,
            tr.t("canvas-empty-hint", &[]),
            FontId::proportional(theme::TEXT_BODY),
            pal().text_faint,
        );
    }

    // 5. Double-click to edit text.
    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let world = canvas.viewport.screen_to_world(pos, origin);
        if let Some(elem) = canvas.element_at(world) {
            interaction.editing_text_elem = Some(elem.id());
        }
    }

    if let Some(editing_id) = interaction.editing_text_elem {
        let info = canvas.get_element(editing_id).map(|e| {
            let text = match e {
                CanvasElement::StickyNote { text, .. } | CanvasElement::Shape { text, .. } => {
                    text.clone()
                }
                CanvasElement::Connector { label, .. } => label.clone(),
                _ => String::new(),
            };
            (e.bounding_rect(), text)
        });
        match info {
            Some((bounds, mut text_buf)) => {
                let s_rect = canvas.viewport.world_rect_to_screen(bounds, origin);
                let mut close_edit = ctx.input(|i| i.key_pressed(egui::Key::Escape));
                let mut changed = false;
                egui::Area::new(Id::new("canvas_inline_text_edit_area"))
                    .fixed_pos(s_rect.min)
                    .order(egui::Order::Foreground)
                    .show(&ctx, |ui| {
                        theme::popover_frame()
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_max_width(s_rect.width().max(220.0));
                                let resp = ui.add(
                                    egui::TextEdit::multiline(&mut text_buf)
                                        .id(Id::new("canvas_inline_text_edit"))
                                        .desired_width(s_rect.width().max(200.0))
                                        .desired_rows(3),
                                );
                                if !resp.has_focus() && !resp.lost_focus() {
                                    resp.request_focus();
                                }
                                changed = resp.changed();
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(tr.t("canvas-edit-hint", &[]))
                                            .size(theme::TEXT_XS)
                                            .color(pal().text_faint),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if widgets::primary_button(
                                                ui,
                                                None,
                                                &tr.t("canvas-edit-done", &[]),
                                            )
                                            .clicked()
                                            {
                                                close_edit = true;
                                            }
                                        },
                                    );
                                });
                            });
                    });
                if changed {
                    modified = true;
                    if let Some(elem) = canvas.get_element_mut(editing_id) {
                        match elem {
                            CanvasElement::StickyNote { text, .. }
                            | CanvasElement::Shape { text, .. } => *text = text_buf,
                            CanvasElement::Connector { label, .. } => *label = text_buf,
                            _ => {}
                        }
                    }
                }
                if close_edit {
                    interaction.editing_text_elem = None;
                }
            }
            None => interaction.editing_text_elem = None,
        }
    }

    // 6. Tool dock (top-left of the canvas).
    egui::Area::new(Id::new("mnemonic_canvas_tool_dock"))
        .fixed_pos(screen_rect.min + Vec2::new(12.0, 12.0))
        .order(egui::Order::Middle)
        .show(&ctx, |ui| {
            match ui::LeftToolbar::show_canvas(ui, tr, interaction.active_tool) {
                Some(ui::LeftToolbarEvent::SelectCanvasTool(tool)) => {
                    interaction.active_tool = tool
                }
                Some(ui::LeftToolbarEvent::ExportDrawio) => {
                    if let Some(save_path) = rfd::FileDialog::new()
                        .add_filter("Draw.io", &["drawio", "xml"])
                        .set_file_name(format!("{}.drawio", canvas.title.replace(' ', "_")))
                        .save_file()
                    {
                        toast = Some(match std::fs::write(&save_path, canvas.to_drawio_xml()) {
                            Ok(()) => (ToastKind::Success, tr.t("canvas-export-success", &[])),
                            Err(e) => (
                                ToastKind::Error,
                                format!("{}: {e}", tr.t("canvas-export-failed", &[])),
                            ),
                        });
                    }
                }
                Some(ui::LeftToolbarEvent::ImportDrawio) => {
                    if let Some(load_path) = rfd::FileDialog::new()
                        .add_filter("Draw.io", &["drawio", "xml"])
                        .pick_file()
                    {
                        let title = load_path
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| canvas.title.clone());
                        let result = std::fs::read_to_string(&load_path)
                            .map_err(anyhow::Error::from)
                            .and_then(|xml| canvas::DrawioImporter::from_xml_with_report(&title, &xml));
                        toast = Some(match result {
                            Ok((imported, _)) if imported.elements.is_empty() => {
                                (ToastKind::Error, tr.t("canvas-import-empty", &[]))
                            }
                            Ok((imported, report)) => {
                                canvas.elements = imported.elements;
                                let bounds = canvas
                                    .elements
                                    .iter()
                                    .map(|e| e.bounding_rect())
                                    .fold(egui::Rect::NOTHING, |acc, r| acc.union(r));
                                canvas.viewport.fit_rect(bounds, screen_rect.size());
                                modified = true;
                                let count = report.elements.to_string();
                                let mut message =
                                    tr.t("canvas-import-success", &[("count", &count)]);
                                if report.skipped > 0 {
                                    let skipped = report.skipped.to_string();
                                    message = format!(
                                        "{message} · {}",
                                        tr.t("canvas-import-skipped", &[("count", &skipped)])
                                    );
                                }
                                (ToastKind::Success, message)
                            }
                            Err(e) => (
                                ToastKind::Error,
                                format!("{}: {e}", tr.t("canvas-import-failed", &[])),
                            ),
                        });
                    }
                }
                None => {}
            }
        });

    // 7. Zoom HUD (bottom-right) and style HUD (bottom-center).
    egui::Area::new(Id::new("mnemonic_canvas_zoom_hud"))
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(screen_rect.right_bottom() - Vec2::new(16.0, 16.0))
        .order(egui::Order::Middle)
        .show(&ctx, |ui| {
            match ui::CanvasHud::show_zoom_hud(ui, tr, canvas.viewport.zoom) {
                Some(ui::CanvasHudEvent::ZoomIn) => {
                    canvas.viewport.zoom = (canvas.viewport.zoom * 1.15).min(5.0);
                    modified = true;
                }
                Some(ui::CanvasHudEvent::ZoomOut) => {
                    canvas.viewport.zoom = (canvas.viewport.zoom / 1.15).max(0.2);
                    modified = true;
                }
                Some(ui::CanvasHudEvent::ResetZoom) => {
                    canvas.viewport.zoom = 1.0;
                    modified = true;
                }
                _ => {}
            }
        });

    egui::Area::new(Id::new("mnemonic_canvas_style_hud"))
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(screen_rect.center_bottom() - Vec2::new(0.0, 16.0))
        .order(egui::Order::Middle)
        .show(&ctx, |ui| {
            match ui::CanvasHud::show_style_hud(
                ui,
                tr,
                interaction.primary_color,
                interaction.stroke_width,
            ) {
                Some(ui::CanvasHudEvent::SetStrokeColor(col)) => interaction.primary_color = col,
                Some(ui::CanvasHudEvent::SetStrokeWidth(w)) => interaction.stroke_width = w,
                _ => {}
            }
        });

    CanvasOutcome { modified, toast }
}
