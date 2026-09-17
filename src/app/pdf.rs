//! PDF viewer (§Fase 8–9): page navigation & zoom, annotation tools
//! (highlight / underline / sticky note / text), page operations (rotate,
//! delete, split, merge), metadata editor, and Save / Export.
//!
//! Page ops always write to a new file (see `pdf::editor`); Save overwrites
//! in place with an automatic backup. Results are reported as toasts.

use std::path::PathBuf;

use anyhow::{Context, Result};
use egui::{Align, Align2, Id, Layout, Margin, RichText, Vec2};
use egui_icons::icons::{
    ICON_ADD, ICON_CALL_SPLIT, ICON_DELETE, ICON_EDIT_NOTE, ICON_FORMAT_UNDERLINED, ICON_HIGHLIGHT,
    ICON_INFO, ICON_IOS_SHARE, ICON_MERGE, ICON_MORE_HORIZ, ICON_NAVIGATE_BEFORE,
    ICON_NAVIGATE_NEXT, ICON_NEAR_ME, ICON_PICTURE_AS_PDF, ICON_REMOVE, ICON_ROTATE_LEFT,
    ICON_ROTATE_RIGHT, ICON_SAVE, ICON_STICKY_NOTE_2, ICON_TEXT_FIELDS,
};
use uuid::Uuid;

use super::MnemonicApp;
use crate::pdf::annotator::{Annotation, AnnotationKind};
use crate::pdf::editor::DocumentMetadata;
use crate::pdf::{self, PdfRenderer, annotator as pdf_annotator, editor as pdf_editor};
use crate::ui::{self, ToastKind, pal, theme, widgets};

/// Rendered page width at 100% zoom, in pixels.
const DEFAULT_PDF_ZOOM_WIDTH: u16 = 900;
const MIN_ZOOM_WIDTH: u16 = 300;
const MAX_ZOOM_WIDTH: u16 = 2700;

/// Default annotation color: a highlighter yellow.
const DEFAULT_ANNOTATION_COLOR: [f32; 3] = [1.0, 0.92, 0.23];

/// Minimum drag distance (px) that counts as drawing a rectangle rather
/// than a click (a click places a default-sized annotation).
const MIN_ANNOTATION_DRAG_PX: f32 = 4.0;

/// Lazily-bound PDFium renderer; a failure is remembered so a missing
/// library isn't retried every frame.
pub(super) enum PdfRendererState {
    Uninit,
    Ready(PdfRenderer),
    Failed(String),
}

pub(super) struct PdfViewerState {
    pub(super) path: PathBuf,
    page_count: usize,
    /// 0-based.
    page_index: usize,
    zoom_width: u16,
    texture: Option<egui::TextureHandle>,
    rendered_key: Option<(usize, u16)>,
    render_error: Option<String>,
    /// Current page size in PDF points, for mapping drags to page space.
    page_size_points: Option<(f32, f32)>,
    split_from: u32,
    split_to: u32,
    delete_confirm: bool,

    annotate_tool: Option<AnnotationKind>,
    annotate_color: [f32; 3],
    drag_start: Option<egui::Pos2>,
    /// Not yet written to any file — baked in on Save/Export.
    staged_annotations: Vec<Annotation>,
    pending_annotation: Option<PendingAnnotation>,
    pending_annotation_text: String,

    show_metadata_editor: bool,
    metadata_title: String,
    metadata_author: String,
    metadata_keywords: String,

    show_save_confirm: bool,
}

impl PdfViewerState {
    pub(super) fn has_dialog_open(&self) -> bool {
        self.delete_confirm
            || self.pending_annotation.is_some()
            || self.show_metadata_editor
            || self.show_save_confirm
    }
}

/// A placed sticky note / text injection waiting for its text.
struct PendingAnnotation {
    kind: AnnotationKind,
    page: u32,
    rect: (f32, f32, f32, f32),
}

enum PdfViewerAction {
    RotateCurrentPage {
        path: PathBuf,
        page: u32,
        degrees: i64,
    },
    DeleteCurrentPage {
        path: PathBuf,
        page: u32,
    },
    Split {
        path: PathBuf,
        from: u32,
        to: u32,
    },
    Merge {
        path: PathBuf,
    },
    SaveOver {
        path: PathBuf,
        annotations: Vec<Annotation>,
        metadata: DocumentMetadata,
    },
    ExportAs {
        path: PathBuf,
        annotations: Vec<Annotation>,
        metadata: DocumentMetadata,
    },
}

impl MnemonicApp {
    fn ensure_pdf_renderer(&mut self) -> Result<&PdfRenderer, &str> {
        if matches!(self.pdf_renderer, PdfRendererState::Uninit) {
            self.pdf_renderer = match PdfRenderer::new() {
                Ok(renderer) => PdfRendererState::Ready(renderer),
                Err(e) => PdfRendererState::Failed(format!("{e:#}")),
            };
        }
        match &self.pdf_renderer {
            PdfRendererState::Ready(renderer) => Ok(renderer),
            PdfRendererState::Failed(msg) => Err(msg.as_str()),
            PdfRendererState::Uninit => unreachable!("resolved just above"),
        }
    }

    pub(super) fn refresh_pdf_documents(&mut self) {
        self.pdf_documents = self
            .index
            .as_ref()
            .and_then(|index| index.list_pdf_documents().ok())
            .unwrap_or_default();
        self.refresh_derived();
    }

    /// Registers an imported PDF and queues it for chunking + embedding.
    pub(super) fn register_pdf(&mut self, path: PathBuf) {
        if let Some(index) = self.index.as_ref()
            && let Err(e) = index.add_pdf_document(&path)
        {
            log::warn!("app: failed to register imported pdf: {e}");
        }
        self.submit_pdf_for_indexing(path);
    }

    /// Opens `path` at its first page. Page count comes from PDFium when
    /// available, falling back to the text extractor.
    pub(super) fn open_pdf(&mut self, path: PathBuf) {
        self.editor = None;
        let page_count = match self.ensure_pdf_renderer() {
            Ok(renderer) => renderer.page_count(&path).ok(),
            Err(_) => None,
        }
        .or_else(|| pdf::extract_pages(&path).ok().map(|pages| pages.len()))
        .unwrap_or(1)
        .max(1);
        let metadata = pdf_editor::get_metadata(&path).unwrap_or_default();

        self.pdf_viewer = Some(PdfViewerState {
            path,
            page_count,
            page_index: 0,
            zoom_width: DEFAULT_PDF_ZOOM_WIDTH,
            texture: None,
            rendered_key: None,
            render_error: None,
            page_size_points: None,
            split_from: 1,
            split_to: page_count as u32,
            delete_confirm: false,
            annotate_tool: None,
            annotate_color: DEFAULT_ANNOTATION_COLOR,
            drag_start: None,
            staged_annotations: Vec::new(),
            pending_annotation: None,
            pending_annotation_text: String::new(),
            show_metadata_editor: false,
            metadata_title: metadata.title,
            metadata_author: metadata.author,
            metadata_keywords: metadata.keywords,
            show_save_confirm: false,
        });
    }

    /// Jump-to-source for search results and chat citations.
    pub(super) fn open_pdf_at_page(&mut self, path: PathBuf, page_index: usize) {
        self.open_pdf(path);
        if let Some(viewer) = self.pdf_viewer.as_mut() {
            viewer.page_index = page_index.min(viewer.page_count.saturating_sub(1));
        }
    }

    fn apply_pdf_viewer_action(&mut self, action: PdfViewerAction) {
        let pdf_dialog = || rfd::FileDialog::new().add_filter("PDF", &["pdf"]);
        let result = match &action {
            PdfViewerAction::RotateCurrentPage {
                path,
                page,
                degrees,
            } => pdf_dialog()
                .save_file()
                .map(|out| pdf_editor::rotate(path, &[*page], *degrees, &out).map(|()| out)),
            PdfViewerAction::DeleteCurrentPage { path, page } => pdf_dialog()
                .save_file()
                .map(|out| pdf_editor::delete_pages(path, &[*page], &out).map(|()| out)),
            PdfViewerAction::Split { path, from, to } => {
                let pages: Vec<u32> = (*from.min(to)..=*from.max(to)).collect();
                pdf_dialog()
                    .save_file()
                    .map(|out| pdf_editor::split(path, &pages, &out).map(|()| out))
            }
            PdfViewerAction::Merge { path } => pdf_dialog().pick_file().and_then(|other| {
                pdf_dialog().save_file().map(|out| {
                    pdf_editor::merge(&[path.as_path(), other.as_path()], &out).map(|()| out)
                })
            }),
            PdfViewerAction::SaveOver {
                path,
                annotations,
                metadata,
            } => Some(
                bake_pdf_changes(path, annotations, metadata).and_then(|staged| {
                    let backup = pdf_editor::save_over(path, &staged)?;
                    let _ = std::fs::remove_file(&staged);
                    Ok(backup)
                }),
            ),
            PdfViewerAction::ExportAs {
                path,
                annotations,
                metadata,
            } => pdf_dialog().save_file().map(|target| {
                bake_pdf_changes(path, annotations, metadata).and_then(|staged| {
                    std::fs::copy(&staged, &target)
                        .with_context(|| format!("copying staged PDF to {}", target.display()))?;
                    let _ = std::fs::remove_file(&staged);
                    Ok(target)
                })
            }),
        };

        match result {
            Some(Ok(output)) => {
                if matches!(action, PdfViewerAction::SaveOver { .. }) {
                    if let Some(viewer) = self.pdf_viewer.as_mut() {
                        viewer.staged_annotations.clear();
                        viewer.rendered_key = None;
                    }
                    self.toast(
                        ToastKind::Success,
                        "pdf-save-success",
                        &[("backup", &output.display().to_string())],
                    );
                } else {
                    self.register_pdf(output.clone());
                    self.refresh_pdf_documents();
                    self.open_pdf(output);
                    self.toast(ToastKind::Success, "pdf-op-success", &[]);
                }
            }
            Some(Err(e)) => self.report_error("pdf-op-error", e),
            None => {} // file dialog cancelled
        }
    }

    pub(super) fn show_pdf_viewer(&mut self, ui: &mut egui::Ui) {
        let Some(mut viewer) = self.pdf_viewer.take() else {
            return;
        };

        let key = (viewer.page_index, viewer.zoom_width);
        if viewer.rendered_key != Some(key) {
            match self.ensure_pdf_renderer() {
                Ok(renderer) => {
                    viewer.page_size_points = renderer
                        .page_size_points(&viewer.path, viewer.page_index)
                        .ok();
                    match renderer.render_page(&viewer.path, viewer.page_index, viewer.zoom_width) {
                        Ok(page) => {
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [page.width, page.height],
                                &page.rgba,
                            );
                            viewer.texture = Some(ui.ctx().load_texture(
                                "pdf-page",
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                            viewer.render_error = None;
                        }
                        Err(e) => {
                            viewer.texture = None;
                            viewer.render_error = Some(format!("{e:#}"));
                        }
                    }
                }
                Err(msg) => {
                    viewer.texture = None;
                    viewer.render_error = Some(msg.to_string());
                    viewer.page_size_points = None;
                }
            }
            viewer.rendered_key = Some(key);
        }

        let tr = &self.locales;
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let mut actions: Vec<PdfViewerAction> = Vec::new();
        let mut export_requested = false;
        let path = viewer.path.clone();
        let page_count = viewer.page_count;

        // Arrow keys / PageUp / PageDown turn pages while nothing is focused.
        if !ui.ctx().egui_wants_keyboard_input() && !viewer.has_dialog_open() {
            let (next, prev) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::PageDown),
                    i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::PageUp),
                )
            });
            if next && viewer.page_index + 1 < page_count {
                viewer.page_index += 1;
            }
            if prev && viewer.page_index > 0 {
                viewer.page_index -= 1;
            }
        }

        // ── Toolbar ──
        egui::Panel::top("pdf_toolbar")
            .frame(theme::top_bar_frame().inner_margin(Margin::symmetric(12, 6)))
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add_enabled_ui(viewer.page_index > 0, |ui| {
                        if widgets::icon_button(
                            ui,
                            ICON_NAVIGATE_BEFORE.codepoint,
                            &format!("{}  ←", t("pdf-prev-page")),
                            false,
                        )
                        .clicked()
                        {
                            viewer.page_index -= 1;
                        }
                    });
                    ui.label(
                        RichText::new(tr.t(
                            "pdf-page-of",
                            &[
                                ("current", &(viewer.page_index + 1).to_string()),
                                ("total", &page_count.to_string()),
                            ],
                        ))
                        .size(theme::TEXT_SM)
                        .color(p.text_dim),
                    );
                    ui.add_enabled_ui(viewer.page_index + 1 < page_count, |ui| {
                        if widgets::icon_button(
                            ui,
                            ICON_NAVIGATE_NEXT.codepoint,
                            &format!("{}  →", t("pdf-next-page")),
                            false,
                        )
                        .clicked()
                        {
                            viewer.page_index += 1;
                        }
                    });

                    ui.separator();
                    if widgets::icon_button(ui, ICON_REMOVE.codepoint, &t("canvas-zoom-out"), false)
                        .clicked()
                    {
                        viewer.zoom_width =
                            ((viewer.zoom_width as f32 / 1.2) as u16).max(MIN_ZOOM_WIDTH);
                    }
                    let pct =
                        (viewer.zoom_width as f32 / DEFAULT_PDF_ZOOM_WIDTH as f32 * 100.0).round();
                    if ui
                        .add(
                            egui::Label::new(
                                RichText::new(format!("{pct}%"))
                                    .size(theme::TEXT_SM)
                                    .color(p.text_dim),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text(t("canvas-zoom-reset"))
                        .clicked()
                    {
                        viewer.zoom_width = DEFAULT_PDF_ZOOM_WIDTH;
                    }
                    if widgets::icon_button(ui, ICON_ADD.codepoint, &t("canvas-zoom-in"), false)
                        .clicked()
                    {
                        viewer.zoom_width =
                            ((viewer.zoom_width as f32 * 1.2) as u16).min(MAX_ZOOM_WIDTH);
                    }

                    ui.separator();
                    let tools = [
                        (None, ICON_NEAR_ME.codepoint, "pdf-annotate-none"),
                        (
                            Some(AnnotationKind::Highlight),
                            ICON_HIGHLIGHT.codepoint,
                            "pdf-annotate-highlight",
                        ),
                        (
                            Some(AnnotationKind::Underline),
                            ICON_FORMAT_UNDERLINED.codepoint,
                            "pdf-annotate-underline",
                        ),
                        (
                            Some(AnnotationKind::StickyNote),
                            ICON_STICKY_NOTE_2.codepoint,
                            "pdf-annotate-sticky",
                        ),
                        (
                            Some(AnnotationKind::TextInjection),
                            ICON_TEXT_FIELDS.codepoint,
                            "pdf-annotate-text",
                        ),
                    ];
                    for (tool, icon, key) in tools {
                        if widgets::icon_button(ui, icon, &t(key), viewer.annotate_tool == tool)
                            .clicked()
                        {
                            viewer.annotate_tool = tool;
                        }
                    }
                    ui.color_edit_button_rgb(&mut viewer.annotate_color)
                        .on_hover_text(t("pdf-annotate-color"));
                    if !viewer.staged_annotations.is_empty() {
                        ui.label(
                            RichText::new(tr.t(
                                "pdf-unsaved-annotations",
                                &[("count", &viewer.staged_annotations.len().to_string())],
                            ))
                            .size(theme::TEXT_XS)
                            .color(p.warning),
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::primary_button(ui, Some(ICON_SAVE.codepoint), &t("pdf-save"))
                            .clicked()
                        {
                            viewer.show_save_confirm = true;
                        }
                        if widgets::icon_button(
                            ui,
                            ICON_IOS_SHARE.codepoint,
                            &t("pdf-export"),
                            false,
                        )
                        .clicked()
                        {
                            export_requested = true;
                        }
                        let more = widgets::icon_button(
                            ui,
                            ICON_MORE_HORIZ.codepoint,
                            &t("pdf-more"),
                            false,
                        );
                        egui::Popup::menu(&more)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                ui.set_min_width(270.0);
                                widgets::section_header(ui, &t("pdf-section-page"));
                                let page = (viewer.page_index + 1) as u32;
                                if widgets::menu_item(
                                    ui,
                                    ICON_ROTATE_LEFT.codepoint,
                                    &t("pdf-rotate-left"),
                                    None,
                                )
                                .clicked()
                                {
                                    actions.push(PdfViewerAction::RotateCurrentPage {
                                        path: path.clone(),
                                        page,
                                        degrees: -90,
                                    });
                                    ui.close();
                                }
                                if widgets::menu_item(
                                    ui,
                                    ICON_ROTATE_RIGHT.codepoint,
                                    &t("pdf-rotate-right"),
                                    None,
                                )
                                .clicked()
                                {
                                    actions.push(PdfViewerAction::RotateCurrentPage {
                                        path: path.clone(),
                                        page,
                                        degrees: 90,
                                    });
                                    ui.close();
                                }
                                if widgets::menu_item_colored(
                                    ui,
                                    ICON_DELETE.codepoint,
                                    &t("pdf-delete-page"),
                                    None,
                                    p.danger,
                                )
                                .clicked()
                                {
                                    viewer.delete_confirm = true;
                                    ui.close();
                                }

                                widgets::section_header(ui, &t("pdf-section-document"));
                                ui.horizontal(|ui| {
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(ICON_CALL_SPLIT.codepoint)
                                            .size(16.0)
                                            .color(p.text_dim),
                                    );
                                    ui.label(t("pdf-split"));
                                    ui.add(
                                        egui::DragValue::new(&mut viewer.split_from)
                                            .range(1..=page_count as u32),
                                    );
                                    ui.label(t("pdf-split-to"));
                                    ui.add(
                                        egui::DragValue::new(&mut viewer.split_to)
                                            .range(1..=page_count as u32),
                                    );
                                });
                                ui.horizontal(|ui| {
                                    ui.add_space(8.0);
                                    if widgets::secondary_button(ui, None, &t("pdf-split-go"))
                                        .clicked()
                                    {
                                        actions.push(PdfViewerAction::Split {
                                            path: path.clone(),
                                            from: viewer.split_from,
                                            to: viewer.split_to,
                                        });
                                        ui.close();
                                    }
                                });
                                if widgets::menu_item(
                                    ui,
                                    ICON_MERGE.codepoint,
                                    &t("pdf-merge"),
                                    None,
                                )
                                .clicked()
                                {
                                    actions.push(PdfViewerAction::Merge { path: path.clone() });
                                    ui.close();
                                }
                                if widgets::menu_item(
                                    ui,
                                    ICON_INFO.codepoint,
                                    &t("pdf-metadata-button"),
                                    None,
                                )
                                .clicked()
                                {
                                    viewer.show_metadata_editor = true;
                                    ui.close();
                                }
                            });
                    });
                });
            });

        // ── Page ──
        let page_size_points = viewer.page_size_points;
        let texture = viewer.texture.clone();
        let render_error = viewer.render_error.clone();
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(err) = &render_error {
                    widgets::empty_state(
                        ui,
                        ICON_PICTURE_AS_PDF.codepoint,
                        &t("pdf-render-unavailable"),
                        err,
                        None,
                    );
                    return;
                }
                let Some(tex) = &texture else { return };
                ui.add_space(theme::SPACE_XL);
                let size = tex.size_vec2();
                let offset = ((ui.available_width() - size.x) / 2.0).max(theme::SPACE_XL);
                ui.horizontal(|ui| {
                    ui.add_space(offset);
                    let sense = if viewer.annotate_tool.is_some() {
                        egui::Sense::click_and_drag()
                    } else {
                        egui::Sense::hover()
                    };
                    let img_response = ui.add(egui::Image::new((tex.id(), size)).sense(sense));
                    let img_rect = img_response.rect;
                    ui.painter().rect_stroke(
                        img_rect,
                        egui::CornerRadius::ZERO,
                        egui::Stroke::new(1.0, p.border),
                        egui::StrokeKind::Outside,
                    );
                    if viewer.annotate_tool.is_some() && img_response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    }
                    let current_page = (viewer.page_index + 1) as u32;

                    let Some((page_w, page_h)) =
                        page_size_points.filter(|(w, h)| *w > 0.0 && *h > 0.0)
                    else {
                        return;
                    };
                    let painter = ui.painter();
                    for annotation in viewer
                        .staged_annotations
                        .iter()
                        .filter(|a| a.page == current_page)
                    {
                        let screen_rect =
                            annotation_screen_rect(annotation.rect, img_rect, page_w, page_h);
                        draw_annotation_overlay(
                            painter,
                            screen_rect,
                            annotation.kind,
                            annotation.color,
                            &annotation.contents,
                        );
                    }

                    let Some(tool) = viewer.annotate_tool else {
                        return;
                    };
                    if img_response.drag_started() {
                        viewer.drag_start = img_response.interact_pointer_pos();
                    }
                    let c = viewer.annotate_color;
                    let stroke_color = egui::Color32::from_rgb(
                        (c[0] * 255.0) as u8,
                        (c[1] * 255.0) as u8,
                        (c[2] * 255.0) as u8,
                    );
                    if img_response.dragged()
                        && let (Some(start), Some(current)) =
                            (viewer.drag_start, img_response.interact_pointer_pos())
                    {
                        painter.rect_stroke(
                            egui::Rect::from_two_pos(start, current).intersect(img_rect),
                            egui::CornerRadius::ZERO,
                            (2.0, stroke_color),
                            egui::StrokeKind::Middle,
                        );
                    }
                    let released = img_response.drag_stopped() || img_response.clicked();
                    if released
                        && let Some(start) =
                            viewer.drag_start.or(img_response.interact_pointer_pos())
                    {
                        let end = img_response.interact_pointer_pos().unwrap_or(start);
                        let mut screen_rect =
                            egui::Rect::from_two_pos(start, end).intersect(img_rect);
                        if screen_rect.width() < MIN_ANNOTATION_DRAG_PX
                            && screen_rect.height() < MIN_ANNOTATION_DRAG_PX
                        {
                            screen_rect =
                                egui::Rect::from_min_size(start, default_annotation_size_px(tool))
                                    .intersect(img_rect);
                        }
                        if screen_rect.width() > 0.5 && screen_rect.height() > 0.5 {
                            let sx = page_w / img_rect.width();
                            let sy = page_h / img_rect.height();
                            // PDF y grows upward from the bottom; screen y grows downward.
                            let pdf_rect = (
                                (screen_rect.left() - img_rect.left()) * sx,
                                page_h - (screen_rect.bottom() - img_rect.top()) * sy,
                                (screen_rect.right() - img_rect.left()) * sx,
                                page_h - (screen_rect.top() - img_rect.top()) * sy,
                            );
                            match tool {
                                AnnotationKind::Highlight | AnnotationKind::Underline => {
                                    viewer.staged_annotations.push(Annotation {
                                        kind: tool,
                                        page: current_page,
                                        rect: pdf_rect,
                                        color: (c[0], c[1], c[2]),
                                        contents: String::new(),
                                    });
                                }
                                AnnotationKind::StickyNote | AnnotationKind::TextInjection => {
                                    viewer.pending_annotation = Some(PendingAnnotation {
                                        kind: tool,
                                        page: current_page,
                                        rect: pdf_rect,
                                    });
                                    viewer.pending_annotation_text.clear();
                                }
                            }
                        }
                        viewer.drag_start = None;
                    }
                });
                ui.add_space(theme::SPACE_XL);
            });

        // ── Dialogs ──
        let ctx = ui.ctx().clone();
        let cancel = t("confirm-cancel");
        if viewer.delete_confirm
            && let Some(confirmed) = ui::ConfirmModal::show(
                &ctx,
                &t("pdf-delete-page"),
                &t("pdf-delete-page-confirm"),
                &t("pdf-delete-page"),
                &cancel,
                true,
            )
        {
            viewer.delete_confirm = false;
            if confirmed {
                actions.push(PdfViewerAction::DeleteCurrentPage {
                    path: path.clone(),
                    page: (viewer.page_index + 1) as u32,
                });
            }
        }
        if viewer.show_save_confirm
            && let Some(confirmed) = ui::ConfirmModal::show(
                &ctx,
                &t("pdf-save"),
                &t("pdf-save-confirm"),
                &t("pdf-save"),
                &cancel,
                false,
            )
        {
            viewer.show_save_confirm = false;
            if confirmed {
                actions.push(PdfViewerAction::SaveOver {
                    path: path.clone(),
                    annotations: viewer.staged_annotations.clone(),
                    metadata: current_metadata(&viewer),
                });
            }
        }

        if let Some(kind) = viewer.pending_annotation.as_ref().map(|p| p.kind) {
            let title = match kind {
                AnnotationKind::TextInjection => t("pdf-annotate-text-prompt"),
                _ => t("pdf-annotate-sticky-prompt"),
            };
            let mut decision = None;
            dialog_window(&ctx, "pdf_annotation_text", &title, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut viewer.pending_annotation_text)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                )
                .request_focus();
                ui.add_space(theme::SPACE_M);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::primary_button(ui, None, &t("pdf-annotate-add")).clicked() {
                        decision = Some(true);
                    }
                    if widgets::ghost_button(ui, None, &t("pdf-annotate-cancel")).clicked() {
                        decision = Some(false);
                    }
                });
            });
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                decision = Some(false);
            }
            match decision {
                Some(true) => {
                    if let Some(pending) = viewer.pending_annotation.take() {
                        let c = viewer.annotate_color;
                        viewer.staged_annotations.push(Annotation {
                            kind: pending.kind,
                            page: pending.page,
                            rect: pending.rect,
                            color: (c[0], c[1], c[2]),
                            contents: std::mem::take(&mut viewer.pending_annotation_text),
                        });
                    }
                }
                Some(false) => {
                    viewer.pending_annotation = None;
                    viewer.pending_annotation_text.clear();
                }
                None => {}
            }
        }

        if viewer.show_metadata_editor {
            let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            dialog_window(
                &ctx,
                "pdf_metadata",
                &t("pdf-metadata-window-title"),
                |ui| {
                    egui::Grid::new("pdf_metadata_grid")
                        .num_columns(2)
                        .spacing(Vec2::new(12.0, 10.0))
                        .show(ui, |ui| {
                            for (key, value) in [
                                ("pdf-metadata-field-title", &mut viewer.metadata_title),
                                ("pdf-metadata-field-author", &mut viewer.metadata_author),
                                ("pdf-metadata-field-keywords", &mut viewer.metadata_keywords),
                            ] {
                                ui.label(RichText::new(t(key)).color(p.text_dim));
                                ui.add(
                                    egui::TextEdit::singleline(value)
                                        .desired_width(260.0)
                                        .margin(Margin::symmetric(8, 5)),
                                );
                                ui.end_row();
                            }
                        });
                    ui.add_space(theme::SPACE_S);
                    ui.label(
                        RichText::new(t("pdf-metadata-hint"))
                            .size(theme::TEXT_XS)
                            .color(p.text_faint),
                    );
                    ui.add_space(theme::SPACE_M);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::primary_button(
                            ui,
                            Some(ICON_EDIT_NOTE.codepoint),
                            &t("pdf-metadata-close"),
                        )
                        .clicked()
                        {
                            close = true;
                        }
                    });
                },
            );
            if close {
                viewer.show_metadata_editor = false;
            }
        }

        if export_requested {
            actions.push(PdfViewerAction::ExportAs {
                path: path.clone(),
                annotations: viewer.staged_annotations.clone(),
                metadata: current_metadata(&viewer),
            });
        }

        self.pdf_viewer = Some(viewer);
        for action in actions {
            self.apply_pdf_viewer_action(action);
        }
    }
}

fn current_metadata(viewer: &PdfViewerState) -> DocumentMetadata {
    DocumentMetadata {
        title: viewer.metadata_title.clone(),
        author: viewer.metadata_author.clone(),
        keywords: viewer.metadata_keywords.clone(),
    }
}

/// A small centered dialog with a title, behind a dimmed backdrop.
fn dialog_window(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    widgets::modal_backdrop(ctx, Id::new((id, "backdrop")));
    egui::Window::new(id)
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(420.0)
        .min_width(420.0)
        .max_width(420.0)
        .frame(theme::popover_frame().inner_margin(Margin::same(20)))
        .show(ctx, |ui| {
            ui.label(
                RichText::new(title)
                    .font(theme::semibold(theme::TEXT_LG))
                    .color(pal().text),
            );
            ui.add_space(theme::SPACE_M);
            add_contents(ui);
        });
}

/// Bakes pending annotations + metadata into a temp copy of `source`.
fn bake_pdf_changes(
    source: &std::path::Path,
    annotations: &[Annotation],
    metadata: &DocumentMetadata,
) -> Result<PathBuf> {
    let stage = |suffix: &str| {
        std::env::temp_dir().join(format!("mnemonic-pdf-{}-{suffix}.pdf", Uuid::new_v4()))
    };
    let annotated_path = stage("annotated");
    let with_annotations: PathBuf = if annotations.is_empty() {
        source.to_path_buf()
    } else {
        pdf_annotator::add_annotations(source, annotations, &annotated_path)?;
        annotated_path.clone()
    };

    let staged_meta = stage("meta");
    pdf_editor::set_metadata(&with_annotations, metadata, &staged_meta)?;
    if with_annotations != source {
        let _ = std::fs::remove_file(&with_annotations);
    }
    Ok(staged_meta)
}

/// Maps an annotation's PDF-space rect onto the rendered page image.
fn annotation_screen_rect(
    rect: (f32, f32, f32, f32),
    img_rect: egui::Rect,
    page_w: f32,
    page_h: f32,
) -> egui::Rect {
    let (x0, y0, x1, y1) = rect;
    let sx = img_rect.width() / page_w;
    let sy = img_rect.height() / page_h;
    egui::Rect::from_min_max(
        egui::pos2(
            img_rect.min.x + x0 * sx,
            img_rect.min.y + (page_h - y1) * sy,
        ),
        egui::pos2(
            img_rect.min.x + x1 * sx,
            img_rect.min.y + (page_h - y0) * sy,
        ),
    )
}

fn default_annotation_size_px(kind: AnnotationKind) -> egui::Vec2 {
    match kind {
        AnnotationKind::StickyNote => egui::vec2(18.0, 18.0),
        AnnotationKind::TextInjection => egui::vec2(160.0, 22.0),
        AnnotationKind::Highlight | AnnotationKind::Underline => egui::vec2(80.0, 14.0),
    }
}

/// Draws a staged (not yet saved) annotation over the page.
fn draw_annotation_overlay(
    painter: &egui::Painter,
    screen_rect: egui::Rect,
    kind: AnnotationKind,
    color: (f32, f32, f32),
    contents: &str,
) {
    let color32 = egui::Color32::from_rgb(
        (color.0 * 255.0) as u8,
        (color.1 * 255.0) as u8,
        (color.2 * 255.0) as u8,
    );
    match kind {
        AnnotationKind::Highlight => {
            painter.rect_filled(
                screen_rect,
                egui::CornerRadius::ZERO,
                color32.gamma_multiply(0.35),
            );
        }
        AnnotationKind::Underline => {
            let y = screen_rect.bottom();
            painter.line_segment(
                [
                    egui::pos2(screen_rect.left(), y),
                    egui::pos2(screen_rect.right(), y),
                ],
                (2.0, color32),
            );
        }
        AnnotationKind::StickyNote => {
            painter.rect_filled(screen_rect, 2u8, color32);
            painter.rect_stroke(
                screen_rect,
                2u8,
                (1.0, egui::Color32::BLACK),
                egui::StrokeKind::Middle,
            );
        }
        AnnotationKind::TextInjection => {
            painter.rect_stroke(
                screen_rect,
                egui::CornerRadius::ZERO,
                (1.0, color32),
                egui::StrokeKind::Middle,
            );
            if !contents.is_empty() {
                painter.text(
                    screen_rect.left_top(),
                    egui::Align2::LEFT_TOP,
                    contents,
                    egui::FontId::proportional(12.0),
                    egui::Color32::BLACK,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_rect_round_trips_between_page_and_screen_space() {
        let img = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(600.0, 800.0));
        let (page_w, page_h) = (612.0, 792.0);
        let pdf_rect = (100.0, 200.0, 300.0, 250.0);
        let screen = annotation_screen_rect(pdf_rect, img, page_w, page_h);
        let sx = page_w / img.width();
        let sy = page_h / img.height();
        let back = (
            (screen.left() - img.left()) * sx,
            page_h - (screen.bottom() - img.top()) * sy,
            (screen.right() - img.left()) * sx,
            page_h - (screen.top() - img.top()) * sy,
        );
        for (a, b) in [
            (pdf_rect.0, back.0),
            (pdf_rect.1, back.1),
            (pdf_rect.2, back.2),
            (pdf_rect.3, back.3),
        ] {
            assert!((a - b).abs() < 0.01);
        }
    }
}
