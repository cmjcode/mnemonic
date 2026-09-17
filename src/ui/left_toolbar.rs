//! Canvas tool dock: a slim vertical toolbar floating inside the canvas,
//! with single-letter shortcuts shown in every tooltip.

use egui::{Key, Ui};
use egui_icons::icons::{
    ICON_ADS_CLICK, ICON_ARROW_RIGHT_ALT, ICON_BRUSH, ICON_CIRCLE, ICON_CROP_SQUARE, ICON_DIAMOND,
    ICON_DOWNLOAD, ICON_INK_ERASER, ICON_PAN_TOOL, ICON_RECTANGLE, ICON_STICKY_NOTE_2,
    ICON_UPLOAD_FILE,
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
                        if i == 2 || i == 7 {
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
