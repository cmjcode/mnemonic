//! Canvas tool dock: a slim vertical toolbar floating inside the canvas,
//! with single-letter shortcuts shown in every tooltip.

use egui::{Key, Ui};
use egui_icons::icons::{
    ICON_ACCOUNT_TREE, ICON_ADS_CLICK, ICON_ARROW_RIGHT_ALT, ICON_BRUSH, ICON_CIRCLE, ICON_CODE,
    ICON_CROP_SQUARE, ICON_DATA_OBJECT, ICON_DIAMOND, ICON_DOWNLOAD, ICON_INK_ERASER, ICON_PAN_TOOL,
    ICON_POST_ADD, ICON_RECTANGLE, ICON_SCHEMA, ICON_SHAPES, ICON_STICKY_NOTE_2, ICON_TABLE_CHART,
    ICON_UPLOAD_FILE, ICON_VISIBILITY,
};

use crate::canvas::CanvasTool;
use crate::canvas::element::ShapeKind;
use crate::i18n::LocaleManager;
use crate::ui::theme;
use crate::ui::widgets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftToolbarEvent {
    SelectCanvasTool(CanvasTool),
    ExportDrawio,
    ImportDrawio,
    /// Re-lay out the section boxes as a mind map (§3.9.2).
    TidyMindMap,
    /// Export the whole canvas as Mermaid (§3.9.4).
    ExportMermaid,
    /// Turn the note's ```mermaid fences into editable canvas objects.
    ImportMermaid,
    /// Bring back section boxes removed from the canvas.
    ShowHidden,
}

/// Locale key of each shape kind in the shape menu.
pub fn shape_label_key(kind: ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Rectangle => "canvas-tool-rectangle",
        ShapeKind::RoundedRect => "canvas-tool-rounded",
        ShapeKind::Ellipse => "canvas-tool-ellipse",
        ShapeKind::Diamond => "canvas-tool-diamond",
        ShapeKind::CalloutBubble => "canvas-shape-callout",
        ShapeKind::Stadium => "canvas-shape-stadium",
        ShapeKind::Circle => "canvas-shape-circle",
        ShapeKind::Hexagon => "canvas-shape-hexagon",
        ShapeKind::Cylinder => "canvas-shape-cylinder",
        ShapeKind::Parallelogram => "canvas-shape-parallelogram",
        ShapeKind::Subroutine => "canvas-shape-subroutine",
        ShapeKind::StateStart => "canvas-shape-state-start",
        ShapeKind::StateEnd => "canvas-shape-state-end",
    }
}

/// `(tool, icon, locale key, shortcut key)` for every canvas tool.
pub const CANVAS_TOOLS: &[(CanvasTool, &str, &str, Key)] = &[
    (
        CanvasTool::Select,
        ICON_ADS_CLICK.codepoint,
        "canvas-tool-select",
        Key::V,
    ),
    (
        CanvasTool::Pan,
        ICON_PAN_TOOL.codepoint,
        "canvas-tool-pan",
        Key::H,
    ),
    (
        CanvasTool::Section,
        ICON_POST_ADD.codepoint,
        "canvas-tool-section",
        Key::N,
    ),
    (
        CanvasTool::StickyNote,
        ICON_STICKY_NOTE_2.codepoint,
        "canvas-tool-sticky",
        Key::S,
    ),
    (
        CanvasTool::Shape(ShapeKind::Rectangle),
        ICON_RECTANGLE.codepoint,
        "canvas-tool-rectangle",
        Key::R,
    ),
    (
        CanvasTool::Shape(ShapeKind::RoundedRect),
        ICON_CROP_SQUARE.codepoint,
        "canvas-tool-rounded",
        Key::U,
    ),
    (
        CanvasTool::Shape(ShapeKind::Ellipse),
        ICON_CIRCLE.codepoint,
        "canvas-tool-ellipse",
        Key::O,
    ),
    (
        CanvasTool::Shape(ShapeKind::Diamond),
        ICON_DIAMOND.codepoint,
        "canvas-tool-diamond",
        Key::D,
    ),
    (
        CanvasTool::Entity,
        ICON_TABLE_CHART.codepoint,
        "canvas-tool-entity",
        Key::T,
    ),
    (
        CanvasTool::ClassBox,
        ICON_DATA_OBJECT.codepoint,
        "canvas-tool-class",
        Key::C,
    ),
    (
        CanvasTool::Connector,
        ICON_ARROW_RIGHT_ALT.codepoint,
        "canvas-tool-connector",
        Key::A,
    ),
    (
        CanvasTool::Pen,
        ICON_BRUSH.codepoint,
        "canvas-tool-pen",
        Key::P,
    ),
    (
        CanvasTool::Eraser,
        ICON_INK_ERASER.codepoint,
        "canvas-tool-eraser",
        Key::E,
    ),
];

/// The tool whose single-letter shortcut was pressed this frame, if any.
/// Callers must skip this while a text field has keyboard focus.
pub fn tool_shortcut_pressed(ctx: &egui::Context) -> Option<CanvasTool> {
    ctx.input(|i| {
        if i.modifiers.any() {
            return None;
        }
        CANVAS_TOOLS
            .iter()
            .find(|(_, _, _, key)| i.key_pressed(*key))
            .map(|(tool, ..)| *tool)
    })
}

pub struct LeftToolbar;

impl LeftToolbar {
    /// Renders the floating tool dock for the canvas.
    pub fn show_canvas(
        ui: &mut Ui,
        tr: &LocaleManager,
        active_tool: CanvasTool,
    ) -> Option<LeftToolbarEvent> {
        let t = |key: &str| tr.t(key, &[]);
        let mut event = None;

        theme::popover_frame()
            .inner_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.vertical(|ui| {
                    for (i, (tool, icon, key, shortcut)) in CANVAS_TOOLS.iter().enumerate() {
                        if i == 2 || i == 8 || i == 11 {
                            ui.add(egui::Separator::default().spacing(6.0));
                        }
                        let tip = format!("{}  {}", t(key), shortcut.name());
                        if widgets::icon_button_sized(
                            ui,
                            icon,
                            &tip,
                            active_tool == *tool,
                            34.0,
                            18.0,
                        )
                        .clicked()
                        {
                            event = Some(LeftToolbarEvent::SelectCanvasTool(*tool));
                        }
                        // Every other shape (flowchart / state vocabulary).
                        if *tool == CanvasTool::Shape(ShapeKind::Diamond) {
                            let more_active = matches!(active_tool, CanvasTool::Shape(k)
                                if !matches!(k, ShapeKind::Rectangle | ShapeKind::RoundedRect | ShapeKind::Ellipse | ShapeKind::Diamond));
                            let resp = widgets::icon_button_sized(
                                ui,
                                ICON_SHAPES.codepoint,
                                &t("canvas-tool-more-shapes"),
                                more_active,
                                34.0,
                                18.0,
                            );
                            egui::Popup::menu(&resp).show(|ui| {
                                for kind in ShapeKind::ALL {
                                    if ui.selectable_label(active_tool == CanvasTool::Shape(kind), t(shape_label_key(kind))).clicked() {
                                        event = Some(LeftToolbarEvent::SelectCanvasTool(CanvasTool::Shape(kind)));
                                    }
                                }
                            });
                        }
                    }
                    ui.add(egui::Separator::default().spacing(6.0));
                    for (icon, key, ev) in [
                        (ICON_ACCOUNT_TREE.codepoint, "canvas-tidy-mindmap", LeftToolbarEvent::TidyMindMap),
                        (ICON_VISIBILITY.codepoint, "canvas-show-hidden", LeftToolbarEvent::ShowHidden),
                        (ICON_SCHEMA.codepoint, "canvas-import-mermaid", LeftToolbarEvent::ImportMermaid),
                        (ICON_CODE.codepoint, "canvas-export-mermaid", LeftToolbarEvent::ExportMermaid),
                    ] {
                        if widgets::icon_button_sized(ui, icon, &t(key), false, 34.0, 18.0).clicked() {
                            event = Some(ev);
                        }
                    }
                    ui.add(egui::Separator::default().spacing(6.0));
                    if widgets::icon_button_sized(
                        ui,
                        ICON_UPLOAD_FILE.codepoint,
                        &t("canvas-import-drawio"),
                        false,
                        34.0,
                        18.0,
                    )
                    .clicked()
                    {
                        event = Some(LeftToolbarEvent::ImportDrawio);
                    }
                    if widgets::icon_button_sized(
                        ui,
                        ICON_DOWNLOAD.codepoint,
                        &t("canvas-export-drawio"),
                        false,
                        34.0,
                        18.0,
                    )
                    .clicked()
                    {
                        event = Some(LeftToolbarEvent::ExportDrawio);
                    }
                });
            });

        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_unique_shortcut() {
        let mut keys: Vec<_> = CANVAS_TOOLS.iter().map(|(_, _, _, k)| k.name()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), CANVAS_TOOLS.len());
    }
}
