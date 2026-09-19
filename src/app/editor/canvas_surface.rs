//! The infinite canvas surface for Canvas and Split modes (§Fase 3, §3.9):
//! tool drags (boxes that become Markdown sections, sticky notes, shapes,
//! ER entities, class boxes, connectors that snap to nodes), moving nodes
//! with their connectors attached, inline editing (`canvas_edit`) with
//! write-back to the Markdown, orphan marking, the tool dock (mind-map
//! tidy, Mermaid and Draw.io import/export) and the zoom/style HUDs.
//! Callers: `app::editor::show_editor`.

use std::collections::HashSet;

use egui::{Align2, FontId, Id, Vec2};

use super::canvas_edit::show_edit_popover;
use crate::canvas::{
    self, BlockBinding, CanvasDocument, CanvasElement, CanvasElementId, CanvasTool, InteractionState, outline, tools,
};
use crate::i18n::LocaleManager;
use crate::markdown::MarkdownEditor;
use crate::ui::{self, ToastKind, pal, theme};

/// Editor-level requests the surface can't carry out itself.
pub(in crate::app) enum CanvasAction {
    /// Re-lay out the section boxes as a mind map.
    Tidy,
    ShowHidden,
    /// Turn the note's ```mermaid fences into canvas objects.
    ImportMermaid { at: [f32; 2] },
    /// A new section box at this world position.
    AddSection { at: [f32; 2] },
    DeleteFromNote(CanvasElementId),
}

/// What the canvas surface asks the app to do after rendering.
pub(in crate::app) struct CanvasOutcome {
    pub(in crate::app) modified: bool,
    /// A Draw.io file replaced the canvas contents: `imported_blocks` are
    /// the text vertices that became bound nodes and need their paragraphs
    /// appended to the Markdown (`MarkdownEditor::import_bound_canvas`).
    pub(in crate::app) imported: Option<(CanvasDocument, Vec<(BlockBinding, String)>)>,
    /// The user asked to bind / unbind the element being text-edited.
    pub(in crate::app) bind: Option<(CanvasElementId, bool)>,
    /// Text of this element changed; `true` = the editor closed.
    pub(in crate::app) text_edited: Option<(CanvasElementId, bool)>,
    /// Bindings of boxes the eraser removed (kept out of the outline).
    pub(in crate::app) erased: Vec<BlockBinding>,
    /// Block id of a bound box the user clicked (Split scrolls to it).
    pub(in crate::app) selected_block: Option<String>,
    pub(in crate::app) action: Option<CanvasAction>,
    pub(in crate::app) toast: Option<(ToastKind, String)>,
}

/// Default world size of a newly drawn element per tool.
fn min_size(tool: CanvasTool) -> [f32; 2] {
    match tool {
        CanvasTool::Entity => [220.0, 120.0],
        CanvasTool::ClassBox => [220.0, 130.0],
        CanvasTool::Shape(k) if k.is_state_marker() => [28.0, 28.0],
        CanvasTool::Shape(_) => [140.0, 80.0],
        _ => [180.0, 120.0],
    }
}

/// Renders the interactive infinite canvas (Canvas mode). Tool docks and
/// HUDs are positioned inside the canvas rect so they never overlap the
/// sidebar or other panels.
pub(in crate::app) fn show_canvas_surface(
    canvas: &mut CanvasDocument,
    interaction: &mut InteractionState,
    orphans: &HashSet<CanvasElementId>,
    ui: &mut egui::Ui,
    is_dark: bool,
    tr: &LocaleManager,
) -> CanvasOutcome {
    let (response, painter) = ui.allocate_painter(ui.available_size_before_wrap(), egui::Sense::click_and_drag());
    let screen_rect = response.rect;
    let origin = screen_rect.min;
    let mut out = CanvasOutcome {
        modified: false,
        imported: None,
        bind: None,
        text_edited: None,
        erased: Vec::new(),
        selected_block: None,
        action: None,
        toast: None,
    };
    let ctx = ui.ctx().clone();

    // 0. Deferred fit-to-content (a newly opened / re-laid-out diagram).
    if std::mem::take(&mut interaction.pending_fit) {
        let bounds = canvas.elements.iter().map(|e| e.bounding_rect()).fold(egui::Rect::NOTHING, |acc, r| acc.union(r));
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
    // isn't part of the saved body, so these don't set `modified`.
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
            canvas.viewport.add_pan_vec(scroll_delta / canvas.viewport.zoom);
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
            let zoom = canvas.viewport.zoom;
            match interaction.active_tool {
                CanvasTool::Pan => canvas.viewport.add_pan_vec(drag_delta / zoom),
                CanvasTool::Pen => interaction.current_freehand_points.push([world_pos.x, world_pos.y]),
                // Moving an element only syncs once, on drag stop; its
                // connectors follow every frame.
                CanvasTool::Select => match interaction.dragged_elem {
                    Some(elem_id) => {
                        let is_node = canvas.get_element(elem_id).is_some_and(|e| e.is_node());
                        if let Some(target) = canvas.get_element_mut(elem_id) {
                            target.translate(drag_delta / zoom);
                        }
                        if is_node {
                            outline::reattach_connectors(canvas, Some(elem_id));
                        }
                    }
                    None => canvas.viewport.add_pan_vec(drag_delta / zoom),
                },
                _ => {}
            }
        }
    } else if response.drag_stopped() {
        if let (Some(start), Some(curr)) = (interaction.drag_start_world, interaction.drag_current_world) {
            finish_drag(canvas, interaction, start, curr, tr, &mut out);
        }
        interaction.is_dragging = false;
        interaction.dragged_elem = None;
        interaction.drag_start_world = None;
        interaction.drag_current_world = None;
    }

    // Clicks: eraser removes, a Section/Entity/Class tool click places a
    // default-sized element, a Select click on a bound box syncs Split.
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let world = canvas.viewport.screen_to_world(pos, origin);
        match interaction.active_tool {
            CanvasTool::Eraser => {
                if let Some(id) = canvas.element_at(world).map(|e| e.id()) {
                    erase(canvas, id, &mut out);
                }
            }
            CanvasTool::Section | CanvasTool::Entity | CanvasTool::ClassBox | CanvasTool::StickyNote | CanvasTool::Shape(_) => {
                let s = min_size(interaction.active_tool);
                finish_drag(canvas, interaction, [world.x, world.y], [world.x + s[0], world.y + s[1]], tr, &mut out);
            }
            CanvasTool::Select => {
                out.selected_block = canvas
                    .element_at(world)
                    .and_then(|e| e.binding())
                    .filter(|b| b.file.is_none())
                    .map(|b| b.block_id.clone());
            }
            _ => {}
        }
    }

    // 4. Elements.
    let hovered_id = response.hover_pos().and_then(|pos| {
        let world = canvas.viewport.screen_to_world(pos, origin);
        canvas.element_at(world).map(|e| e.id())
    });
    // Skip elements entirely off-screen; the margin keeps connector labels and
    // selection handles that poke past an element's bounds from popping.
    let visible_world = canvas.viewport.screen_rect_to_world(screen_rect.expand(64.0), origin);
    for elem in canvas.elements.iter().filter(|e| visible_world.intersects(e.bounding_rect())) {
        canvas::draw_element(&painter, &canvas.viewport, origin, elem, hovered_id == Some(elem.id()), is_dark);
        if orphans.contains(&elem.id()) {
            let r = canvas.viewport.world_rect_to_screen(elem.bounding_rect(), origin).expand(3.0);
            let red = egui::Color32::from_rgb(220, 70, 70);
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            painter.extend(egui::Shape::dashed_line(&pts, egui::Stroke::new(1.5, red), 6.0, 4.0));
            painter.text(
                r.right_top() + Vec2::new(-4.0, -4.0),
                Align2::RIGHT_BOTTOM,
                tr.t("canvas-orphan", &[]),
                FontId::proportional(theme::TEXT_XS),
                red,
            );
        }
    }

    // Preview of the element being drawn.
    if interaction.is_dragging
        && let (Some(s), Some(c)) = (interaction.drag_start_world, interaction.drag_current_world)
        && !matches!(interaction.active_tool, CanvasTool::Select | CanvasTool::Pan | CanvasTool::Pen | CanvasTool::Eraser)
    {
        let a = canvas.viewport.world_to_screen(egui::Pos2::new(s[0], s[1]), origin);
        let b = canvas.viewport.world_to_screen(egui::Pos2::new(c[0], c[1]), origin);
        let stroke = egui::Stroke::new(1.2, pal().accent);
        if interaction.active_tool == CanvasTool::Connector {
            painter.line_segment([a, b], stroke);
        } else {
            painter.rect_stroke(egui::Rect::from_two_pos(a, b), 4.0, stroke, egui::StrokeKind::Middle);
        }
    }

    if interaction.active_tool == CanvasTool::Pen && interaction.current_freehand_points.len() >= 2 {
        let c = interaction.primary_color;
        let stroke_c = egui::Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
        let points: Vec<egui::Pos2> = interaction
            .current_freehand_points
            .iter()
            .map(|pt| canvas.viewport.world_to_screen(egui::Pos2::new(pt[0], pt[1]), origin))
            .collect();
        for w in points.windows(2) {
            painter.line_segment([w[0], w[1]], (interaction.stroke_width * canvas.viewport.zoom, stroke_c));
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
        let s_rect = canvas
            .get_element(editing_id)
            .map(|e| canvas.viewport.world_rect_to_screen(e.bounding_rect(), origin));
        match s_rect.and_then(|r| show_edit_popover(&ctx, canvas, editing_id, r.min, r.width(), tr)) {
            Some(res) => {
                if res.text_changed || res.meta_changed {
                    out.modified = true;
                }
                if res.text_changed {
                    out.text_edited = Some((editing_id, false));
                }
                if let Some(bind) = res.bind {
                    out.bind = Some((editing_id, bind));
                }
                if res.delete_from_note {
                    out.action = Some(CanvasAction::DeleteFromNote(editing_id));
                }
                if res.close {
                    interaction.editing_text_elem = None;
                    if !res.delete_from_note {
                        out.text_edited = Some((editing_id, true));
                    }
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
            let center = canvas.viewport.screen_to_world(screen_rect.center(), origin);
            match ui::LeftToolbar::show_canvas(ui, tr, interaction.active_tool) {
                Some(ui::LeftToolbarEvent::SelectCanvasTool(tool)) => interaction.active_tool = tool,
                Some(ui::LeftToolbarEvent::TidyMindMap) => out.action = Some(CanvasAction::Tidy),
                Some(ui::LeftToolbarEvent::ShowHidden) => out.action = Some(CanvasAction::ShowHidden),
                Some(ui::LeftToolbarEvent::ImportMermaid) => {
                    out.action = Some(CanvasAction::ImportMermaid { at: [center.x, center.y] })
                }
                Some(ui::LeftToolbarEvent::ExportMermaid) => {
                    out.toast = super::canvas_io::export_mermaid(ui.ctx(), canvas, tr)
                }
                Some(ui::LeftToolbarEvent::ExportDrawio) => out.toast = super::canvas_io::export_drawio(canvas, tr),
                Some(ui::LeftToolbarEvent::ImportDrawio) => {
                    if let Some((doc, blocks, toast)) = super::canvas_io::import_drawio(canvas, screen_rect.size(), tr) {
                        out.imported = doc.map(|d| (d, blocks));
                        out.toast = Some(toast);
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
        .show(&ctx, |ui| match ui::CanvasHud::show_zoom_hud(ui, tr, canvas.viewport.zoom) {
            Some(ui::CanvasHudEvent::ZoomIn) => canvas.viewport.zoom = (canvas.viewport.zoom * 1.15).min(5.0),
            Some(ui::CanvasHudEvent::ZoomOut) => canvas.viewport.zoom = (canvas.viewport.zoom / 1.15).max(0.2),
            Some(ui::CanvasHudEvent::ResetZoom) => canvas.viewport.zoom = 1.0,
            _ => {}
        });

    egui::Area::new(Id::new("mnemonic_canvas_style_hud"))
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(screen_rect.center_bottom() - Vec2::new(0.0, 16.0))
        .order(egui::Order::Middle)
        .show(&ctx, |ui| {
            match ui::CanvasHud::show_style_hud(ui, tr, interaction.primary_color, interaction.stroke_width) {
                Some(ui::CanvasHudEvent::SetStrokeColor(col)) => interaction.primary_color = col,
                Some(ui::CanvasHudEvent::SetStrokeWidth(w)) => interaction.stroke_width = w,
                _ => {}
            }
        });

    out
}

/// Removes element `id`, remembering a section box's binding so the
/// outline sync doesn't bring it back.
fn erase(canvas: &mut CanvasDocument, id: CanvasElementId, out: &mut CanvasOutcome) {
    if let Some(elem) = canvas.remove_element(id) {
        if let Some(b) = elem.binding() {
            out.erased.push(b.clone());
        }
        out.modified = true;
    }
}

/// Creates what the active tool draws between `start` and `curr` (world).
fn finish_drag(
    canvas: &mut CanvasDocument,
    interaction: &mut InteractionState,
    start: [f32; 2],
    curr: [f32; 2],
    tr: &LocaleManager,
    out: &mut CanvasOutcome,
) {
    let tool = interaction.active_tool;
    let min = min_size(tool);
    let min_x = start[0].min(curr[0]);
    let min_y = start[1].min(curr[1]);
    let w = (start[0].max(curr[0]) - min_x).max(min[0]);
    let h = (start[1].max(curr[1]) - min_y).max(min[1]);
    let rect = [min_x, min_y, min_x + w, min_y + h];
    let placed: Option<CanvasElement> = match tool {
        CanvasTool::Section => {
            out.action = Some(CanvasAction::AddSection { at: [min_x, min_y] });
            None
        }
        CanvasTool::StickyNote => Some(CanvasElement::StickyNote {
            id: CanvasElementId::new(),
            pos: [min_x, min_y],
            size: [w, h],
            text: tr.t("canvas-new-sticky", &[]),
            color: interaction.primary_color,
            binding: None,
        }),
        CanvasTool::Shape(kind) => {
            let rect = if kind.is_state_marker() { [min_x, min_y, min_x + min[0], min_y + min[1]] } else { rect };
            Some(CanvasElement::Shape {
                id: CanvasElementId::new(),
                kind,
                rect,
                stroke_color: interaction.primary_color,
                stroke_width: interaction.stroke_width,
                fill_color: None,
                text: String::new(),
                text_color: None,
                binding: None,
            })
        }
        CanvasTool::Entity => Some(CanvasElement::Entity {
            id: CanvasElementId::new(),
            rect,
            name: tr.t("canvas-new-entity", &[]).to_uppercase().replace(' ', "_"),
            attributes: vec![canvas::diagram_kinds::EntityAttr {
                ty: "int".into(),
                name: "id".into(),
                keys: "PK".into(),
                comment: String::new(),
            }],
            color: tools::PALETTE_PRIMARY_ACCENT,
        }),
        CanvasTool::ClassBox => Some(CanvasElement::ClassBox {
            id: CanvasElementId::new(),
            rect,
            name: tr.t("canvas-new-class", &[]).replace(' ', ""),
            annotation: String::new(),
            attributes: Vec::new(),
            methods: Vec::new(),
            color: [0.62, 0.52, 0.85],
        }),
        CanvasTool::Connector => {
            // Ends dropped on a node attach to it and follow it around.
            let at = |p: [f32; 2]| {
                canvas
                    .elements
                    .iter()
                    .rev()
                    .filter(|e| e.is_node())
                    .find(|e| e.bounding_rect().contains(egui::Pos2::new(p[0], p[1])))
                    .map(|e| e.id())
            };
            let (from_elem, to_elem) = (at(start), at(curr));
            let id = CanvasElementId::new();
            canvas.add_element(CanvasElement::Connector {
                id,
                from_elem,
                to_elem: to_elem.filter(|t| Some(*t) != from_elem),
                from_pos: start,
                to_pos: curr,
                routing: canvas::ConnectorRouting::Orthogonal,
                stroke_color: interaction.primary_color,
                stroke_width: interaction.stroke_width,
                label: String::new(),
                arrow_end: true,
                waypoints: Vec::new(),
                meta: Default::default(),
            });
            outline::reattach_connectors(canvas, from_elem.or(to_elem));
            interaction.active_tool = CanvasTool::Select;
            out.modified = true;
            None
        }
        CanvasTool::Pen => {
            if interaction.current_freehand_points.len() >= 2 {
                canvas.add_element(CanvasElement::FreehandStroke {
                    id: CanvasElementId::new(),
                    points: std::mem::take(&mut interaction.current_freehand_points),
                    color: interaction.primary_color,
                    width: interaction.stroke_width,
                });
                out.modified = true;
            }
            None
        }
        CanvasTool::Select if interaction.dragged_elem.is_some() => {
            out.modified = true;
            None
        }
        CanvasTool::Eraser => {
            if let Some(id) = canvas.element_at(egui::Pos2::new(start[0], start[1])).map(|e| e.id()) {
                erase(canvas, id, out);
            }
            None
        }
        _ => None,
    };
    if let Some(elem) = placed {
        let id = elem.id();
        canvas.add_element(elem);
        // Entities and classes open straight into editing.
        if matches!(tool, CanvasTool::Entity | CanvasTool::ClassBox) {
            interaction.editing_text_elem = Some(id);
        }
        interaction.active_tool = CanvasTool::Select;
        out.modified = true;
    } else if tool == CanvasTool::Section {
        interaction.active_tool = CanvasTool::Select;
    }
}

/// Applies what the canvas surface asked for to the editor and returns
/// the toast to show, if any.
pub(super) fn apply_canvas_outcome(
    editor: &mut MarkdownEditor,
    outcome: CanvasOutcome,
    tr: &LocaleManager,
) -> Option<(ToastKind, String)> {
    let mut toast = outcome.toast;
    if let Some((doc, blocks)) = outcome.imported {
        editor.import_bound_canvas(doc, blocks);
    } else if outcome.modified {
        editor.sync_canvas_to_body();
    }
    if let Some((id, closing)) = outcome.text_edited {
        editor.write_back_element(id, closing);
    }
    if !outcome.erased.is_empty() {
        editor.hide_segments(&outcome.erased);
    }
    if let Some((id, bind)) = outcome.bind {
        if bind {
            editor.bind_element_to_note(id);
        } else {
            editor.unbind_element(id);
        }
    }
    if let Some(block) = outcome.selected_block {
        editor.scroll_to_block(&block);
    }
    match outcome.action {
        Some(CanvasAction::Tidy) => editor.tidy_canvas(),
        Some(CanvasAction::ShowHidden) => editor.show_hidden_segments(),
        Some(CanvasAction::AddSection { at }) => {
            if let Some(id) = editor.add_section_box("## …", at) {
                editor.canvas_interaction.editing_text_elem = Some(id);
            }
        }
        Some(CanvasAction::DeleteFromNote(id)) => {
            editor.delete_element_from_note(id);
        }
        Some(CanvasAction::ImportMermaid { at }) => toast = Some(super::canvas_io::import_note_mermaid(editor, at, tr)),
        None => {}
    }
    toast
}
