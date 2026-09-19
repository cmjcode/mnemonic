//! The infinite canvas surface for Canvas and Split modes (§Fase 3): tool
//! drags, element drawing, inline text editing with block binding, the tool
//! dock (incl. Draw.io import/export) and the zoom/style HUDs.
//! Callers: `app::editor::show_editor`.

use egui::{Align2, FontId, Id, Margin, RichText, Vec2};
use egui_icons::icons::ICON_LINK;

use crate::canvas::{
    self, BlockBinding, CanvasDocument, CanvasElement, CanvasElementId, CanvasTool, InteractionState,
};
use crate::i18n::LocaleManager;
use crate::markdown::MarkdownEditor;
use crate::ui::{self, ToastKind, pal, theme, widgets};

/// What the canvas surface asks the app to do after rendering.
pub(in crate::app) struct CanvasOutcome {
    pub(in crate::app) modified: bool,
    /// A Draw.io file replaced the canvas contents: `imported_blocks` are
    /// the text vertices that became bound nodes and need their paragraphs
    /// appended to the Markdown (`MarkdownEditor::import_bound_canvas`).
    pub(in crate::app) imported: Option<(CanvasDocument, Vec<(BlockBinding, String)>)>,
    /// The user asked to bind / unbind the element being text-edited.
    pub(in crate::app) bind: Option<(CanvasElementId, bool)>,
    pub(in crate::app) toast: Option<(ToastKind, String)>,
}

/// Renders the interactive infinite canvas (Canvas mode). Tool docks and
/// HUDs are positioned inside the canvas rect so they never overlap the
/// sidebar or other panels.
pub(in crate::app) fn show_canvas_surface(
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
    let mut imported_doc: Option<(CanvasDocument, Vec<(BlockBinding, String)>)> = None;
    let mut bind: Option<(CanvasElementId, bool)> = None;
    let mut toast = None;
    let ctx = ui.ctx().clone();

    // 0. Deferred fit-to-content (set when a Draw.io note is opened: its
    // coordinates can sit anywhere, and the viewport isn't persisted).
    if std::mem::take(&mut interaction.pending_fit) {
        let bounds = canvas
            .elements
            .iter()
            .map(|e| e.bounding_rect())
            .fold(egui::Rect::NOTHING, |acc, r| acc.union(r));
        if bounds.is_positive() {
            canvas.viewport.fit_rect(bounds, screen_rect.size());
        }
    }

    // Single-letter tool shortcuts (V, H, S, R, ...) while nothing is focused.
    if !ctx.egui_wants_keyboard_input()
        && interaction.editing_text_elem.is_none()
        && let Some(tool) = ui::left_toolbar::tool_shortcut_pressed(&ctx)
    {
        interaction.active_tool = tool;
    }

    // 1. Zoom & pan — only while the pointer is over the canvas. The viewport
    // isn't part of the saved body, so these don't set `modified`: doing so
    // re-serialized the whole diagram every frame of a zoom or pan.
    if response.contains_pointer() {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        let zoom_delta = ui.input(|i| i.zoom_delta());
        let ctrl_pressed = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let hover_pos = response.hover_pos();

        if (zoom_delta - 1.0).abs() > 1e-4 {
            if let Some(pos) = hover_pos {
                canvas.viewport.zoom_at(zoom_delta, pos, origin);
            }
        } else if ctrl_pressed && scroll_delta.y != 0.0 {
            let factor = if scroll_delta.y > 0.0 { 1.1 } else { 0.9 };
            if let Some(pos) = hover_pos {
                canvas.viewport.zoom_at(factor, pos, origin);
            }
        } else if scroll_delta != Vec2::ZERO {
            canvas
                .viewport
                .add_pan_vec(scroll_delta / canvas.viewport.zoom);
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
            interaction.dragged_elem = if interaction.active_tool == CanvasTool::Select {
                canvas.element_at(world_pos).map(|e| e.id())
            } else {
                None
            };
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
                }
                CanvasTool::Pen => {
                    interaction
                        .current_freehand_points
                        .push([world_pos.x, world_pos.y]);
                }
                CanvasTool::Select => {
                    // Moving an element only syncs the body once, on drag
                    // stop — not on every frame of the drag.
                    let zoom = canvas.viewport.zoom;
                    match interaction.dragged_elem {
                        Some(elem_id) => {
                            if let Some(target) = canvas.get_element_mut(elem_id) {
                                target.translate(drag_delta / zoom);
                            }
                        }
                        None => canvas.viewport.add_pan_vec(drag_delta / zoom),
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
                        binding: None,
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
                        binding: None,
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
                CanvasTool::Select if interaction.dragged_elem.is_some() => {
                    modified = true;
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
        interaction.dragged_elem = None;
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
    // Skip elements entirely off-screen; the margin keeps connector labels and
    // selection handles that poke past an element's bounds from popping.
    let visible_world = canvas
        .viewport
        .screen_rect_to_world(screen_rect.expand(64.0), origin);
    for elem in canvas
        .elements
        .iter()
        .filter(|e| visible_world.intersects(e.bounding_rect()))
    {
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
            let bindable = matches!(e, CanvasElement::StickyNote { .. } | CanvasElement::Shape { .. });
            (e.bounding_rect(), text, bindable, e.is_bound())
        });
        match info {
            Some((bounds, mut text_buf, bindable, is_bound)) => {
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
                                        RichText::new(if is_bound {
                                            tr.t("canvas-edit-hint-bound", &[])
                                        } else {
                                            tr.t("canvas-edit-hint", &[])
                                        })
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
                                            if bindable {
                                                let label = if is_bound {
                                                    tr.t("canvas-unbind", &[])
                                                } else {
                                                    tr.t("canvas-bind", &[])
                                                };
                                                if widgets::ghost_button(ui, Some(ICON_LINK.codepoint), &label)
                                                    .clicked()
                                                {
                                                    bind = Some((editing_id, !is_bound));
                                                    close_edit = true;
                                                }
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
                        // Bound import (§Fase 3): Draw.io "text" shapes become
                        // Markdown blocks, every other shape stays diagram-only.
                        let result = std::fs::read_to_string(&load_path)
                            .map_err(anyhow::Error::from)
                            .and_then(|xml| canvas::DrawioImporter::from_xml_bound(&title, &xml));
                        toast = Some(match result {
                            Ok((imported, _)) if imported.elements.is_empty() => {
                                (ToastKind::Error, tr.t("canvas-import-empty", &[]))
                            }
                            Ok((mut imported, new_blocks)) => {
                                let bounds = imported
                                    .elements
                                    .iter()
                                    .map(|e| e.bounding_rect())
                                    .fold(egui::Rect::NOTHING, |acc, r| acc.union(r));
                                imported.viewport = canvas.viewport.clone();
                                imported.viewport.fit_rect(bounds, screen_rect.size());
                                let count = imported.elements.len().to_string();
                                let bound = new_blocks.len().to_string();
                                let message = format!(
                                    "{} · {}",
                                    tr.t("canvas-import-success", &[("count", &count)]),
                                    tr.t("canvas-import-bound", &[("count", &bound)])
                                );
                                imported_doc = Some((imported, new_blocks));
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

    CanvasOutcome {
        modified,
        imported: imported_doc,
        bind,
        toast,
    }
}

/// Applies what the canvas surface asked for to the editor and returns
/// the toast to show, if any.
pub(super) fn apply_canvas_outcome(editor: &mut MarkdownEditor, outcome: CanvasOutcome) -> Option<(ToastKind, String)> {
    if let Some((doc, blocks)) = outcome.imported {
        editor.import_bound_canvas(doc, blocks);
    } else if outcome.modified {
        editor.sync_canvas_to_body();
    }
    if let Some((id, bind)) = outcome.bind {
        if bind {
            editor.bind_element_to_note(id);
        } else {
            editor.unbind_element(id);
        }
    }
    outcome.toast
}
