//! Home screen: the document grid for the current library filter or tag,
//! merged keyword + semantic search results, multi-select batch actions,
//! and guiding empty states.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use egui::{Align, Id, Layout, Margin, Rect, RichText, Sense, Stroke, Vec2};
use egui_icons::icons::{
    ICON_AUTO_AWESOME, ICON_CHECK, ICON_CHECK_CIRCLE, ICON_CIRCLE, ICON_CLOSE, ICON_DELETE,
    ICON_DELETE_FOREVER, ICON_DESCRIPTION, ICON_DRAW, ICON_INVENTORY_2, ICON_KEEP, ICON_MORE_HORIZ,
    ICON_NOTE_ADD, ICON_OPEN_IN_NEW, ICON_PALETTE, ICON_PICTURE_AS_PDF, ICON_PUSH_PIN,
    ICON_RESTORE_FROM_TRASH, ICON_SEARCH, ICON_SELECT_ALL, ICON_SORT, ICON_TABLE_CHART, ICON_UPLOAD_FILE,
};
use uuid::Uuid;

use super::{MnemonicApp, ToastAction};
use crate::core::search::{MatchKind, SearchHit};
use crate::core::storage::{HIGHLIGHT_END, HIGHLIGHT_START};

/// Most AI search results listed under the keyword-filtered grid.
/// Height of the diagram thumbnail on a canvas card.
const CANVAS_THUMB_HEIGHT: f32 = 110.0;

const SEARCH_HITS_SHOWN: usize = 8;
use crate::i18n::LocaleManager;
use crate::notes::query::{self, GridFilter, SortMode};
use crate::notes::{Note, Vault};
use crate::ui::{SidebarDocFilter, ToastKind, pal, theme, widgets};

/// Target card width; the grid fits as many columns as the width allows.
const CARD_WIDTH: f32 = 250.0;
const MAX_COLUMNS: usize = 5;
const CONTENT_MAX_WIDTH: f32 = 1400.0;

/// Deferred grid interactions, applied after rendering.
pub(super) enum GridAction {
    Open(PathBuf),
    TogglePin(Uuid),
    SetColor(Uuid, Option<String>),
    ToggleArchived(Uuid),
    Trash(PathBuf),
    Restore(PathBuf),
    ConfirmDelete(Uuid),
    BatchArchive(Vec<Uuid>),
    BatchTrash(Vec<PathBuf>),
    OpenHit(SearchHit),
    /// Trash a non-note file card (PDF or sheet).
    TrashFile(PathBuf),
    NewNote,
    ImportPdf,
    ClearSearch,
    ShowAll,
    EmptyTrash,
}

/// "baru saja", "5 menit lalu", "3 hari lalu", or a short date.
pub(super) fn relative_time(tr: &LocaleManager, then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - then).num_seconds().max(0);
    let n = |v: i64| v.to_string();
    match secs {
        0..60 => tr.t("time-just-now", &[]),
        60..3600 => tr.t("time-minutes-ago", &[("count", &n(secs / 60))]),
        3600..86_400 => tr.t("time-hours-ago", &[("count", &n(secs / 3600))]),
        86_400..604_800 => tr.t("time-days-ago", &[("count", &n(secs / 86_400))]),
        _ => then.format("%d %b %Y").to_string(),
    }
}

fn filter_title_key(filter: &SidebarDocFilter) -> &'static str {
    match filter {
        SidebarDocFilter::All => "sidebar-all",
        SidebarDocFilter::NotesOnly => "sidebar-notes-only",
        SidebarDocFilter::WhiteboardsOnly => "sidebar-whiteboards-only",
        SidebarDocFilter::PdfsOnly => "sidebar-pdfs-only",
        SidebarDocFilter::Archived => "sidebar-archived",
        SidebarDocFilter::Trashed => "sidebar-trash",
        SidebarDocFilter::Tag(_) => "sidebar-tags",
    }
}

impl MnemonicApp {
    pub(super) fn show_grid(&mut self, ui: &mut egui::Ui) {
        let Some(vault) = self.vault.as_ref() else {
            return;
        };
        let tr = &self.locales;
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let now = Utc::now();

        // ── What to show ──
        let filter = self.doc_filter.clone();
        let search = self.search_text.trim().to_string();
        let grid_filter = match &filter {
            SidebarDocFilter::Archived => GridFilter::Archived,
            SidebarDocFilter::Trashed => GridFilter::Trashed,
            SidebarDocFilter::Tag(tag) => GridFilter::Tag(tag.clone()),
            _ => GridFilter::All,
        };
        let mut visible_notes: Vec<&Note> = match &filter {
            SidebarDocFilter::PdfsOnly => Vec::new(),
            _ => query::filter_notes(&vault.notes, &grid_filter, &search),
        };
        match &filter {
            SidebarDocFilter::NotesOnly => visible_notes.retain(|n| !n.is_canvas()),
            SidebarDocFilter::WhiteboardsOnly => visible_notes.retain(|n| n.is_canvas()),
            _ => {}
        }
        query::sort_notes(&mut visible_notes, self.sort_mode);

        let search_lower = search.to_lowercase();
        let name_matches = |p: &&PathBuf| {
            search.is_empty()
                || p.file_name().is_some_and(|n| {
                    n.to_string_lossy().to_lowercase().contains(&search_lower)
                })
        };
        let show_pdfs = matches!(filter, SidebarDocFilter::All | SidebarDocFilter::PdfsOnly);
        let mut visible_files: Vec<&PathBuf> = Vec::new();
        if show_pdfs {
            visible_files.extend(self.pdf_documents.iter().filter(name_matches));
        }
        // Sheets (§3.8) have no filter of their own; they show under "All".
        if filter == SidebarDocFilter::All {
            visible_files.extend(self.derived.sheets.iter().filter(name_matches));
        }

        // Semantic hits for documents the keyword filter didn't already show.
        let shown_ids: HashSet<Uuid> = visible_notes.iter().map(|n| n.frontmatter.id).collect();
        let mut seen_docs = HashSet::new();
        let semantic_hits: Vec<&SearchHit> = if search.is_empty() {
            Vec::new()
        } else {
            self.search_results
                .iter()
                .filter(|h| !shown_ids.contains(&h.chunk.doc_id))
                .filter(|h| {
                    vault
                        .notes
                        .iter()
                        .find(|n| n.frontmatter.id == h.chunk.doc_id)
                        .is_none_or(|n| !n.frontmatter.trashed)
                })
                .filter(|h| seen_docs.insert(h.chunk.doc_id))
                .take(SEARCH_HITS_SHOWN)
                .collect()
        };

        let total = visible_notes.len() + visible_files.len();
        let in_trash = filter == SidebarDocFilter::Trashed;
        let vault_empty = self.derived.counts.all == 0 && self.derived.counts.archived == 0;

        let mut actions: Vec<GridAction> = Vec::new();
        let selection_mode = &mut self.selection_mode;
        let selected = &mut self.selected;
        let sort_mode = &mut self.sort_mode;
        let derived = &self.derived;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let avail = ui.available_width();
                let content_w = (avail - 2.0 * theme::SPACE_XL).min(CONTENT_MAX_WIDTH);
                let margin = ((avail - content_w) / 2.0).max(theme::SPACE_M);
                ui.add_space(theme::SPACE_XL);
                ui.horizontal_top(|ui| {
                    ui.add_space(margin);
                    ui.vertical(|ui| {
                        ui.set_width(content_w);

                        // ── Header ──
                        ui.horizontal(|ui| {
                            let title = match &filter {
                                SidebarDocFilter::Tag(tag) => format!("#{tag}"),
                                other => t(filter_title_key(other)),
                            };
                            ui.label(
                                RichText::new(title)
                                    .font(theme::semibold(theme::TEXT_XL))
                                    .color(p.text),
                            );
                            ui.label(
                                RichText::new(
                                    tr.t("grid-item-count", &[("count", &total.to_string())]),
                                )
                                .size(theme::TEXT_SM)
                                .color(p.text_faint),
                            );
                            if total == 0 {
                                return;
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let sort_btn = widgets::icon_button(
                                    ui,
                                    ICON_SORT.codepoint,
                                    &t("grid-sort"),
                                    false,
                                );
                                egui::Popup::menu(&sort_btn).show(|ui| {
                                    ui.set_min_width(200.0);
                                    widgets::section_header(ui, &t("grid-sort"));
                                    for (mode, key) in [
                                        (SortMode::Modified, "sort-modified"),
                                        (SortMode::Created, "sort-created"),
                                        (SortMode::Title, "sort-title"),
                                        (SortMode::Color, "sort-color"),
                                    ] {
                                        let check =
                                            (*sort_mode == mode).then_some(ICON_CHECK.codepoint);
                                        if widgets::menu_item(ui, "", &t(key), check).clicked() {
                                            *sort_mode = mode;
                                            ui.close();
                                        }
                                    }
                                });
                                if !visible_notes.is_empty()
                                    && widgets::icon_button(
                                        ui,
                                        ICON_SELECT_ALL.codepoint,
                                        &t("selection-mode-on"),
                                        *selection_mode,
                                    )
                                    .clicked()
                                {
                                    *selection_mode = !*selection_mode;
                                    selected.clear();
                                }
                                if in_trash
                                    && widgets::button(
                                        ui,
                                        widgets::ButtonKind::Ghost,
                                        Some(ICON_DELETE_FOREVER.codepoint),
                                        &t("grid-empty-trash"),
                                    )
                                    .clicked()
                                {
                                    actions.push(GridAction::EmptyTrash);
                                }
                            });
                        });

                        if in_trash && total > 0 {
                            ui.add_space(theme::SPACE_S);
                            info_banner(ui, &t("grid-trash-info"));
                        }

                        // ── Selection bar ──
                        if *selection_mode {
                            ui.add_space(theme::SPACE_M);
                            egui::Frame::NONE
                                .fill(p.accent_soft)
                                .corner_radius(theme::RADIUS_LG)
                                .inner_margin(Margin::symmetric(12, 6))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(tr.t(
                                                "selection-count",
                                                &[("count", &selected.len().to_string())],
                                            ))
                                            .color(p.text),
                                        );
                                        if widgets::ghost_button(
                                            ui,
                                            None,
                                            &t("selection-select-all"),
                                        )
                                        .clicked()
                                        {
                                            selected.extend(
                                                visible_notes.iter().map(|n| n.frontmatter.id),
                                            );
                                        }
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if widgets::icon_button(
                                                    ui,
                                                    ICON_CLOSE.codepoint,
                                                    &t("selection-mode-off"),
                                                    false,
                                                )
                                                .clicked()
                                                {
                                                    *selection_mode = false;
                                                    selected.clear();
                                                }
                                                ui.add_enabled_ui(
                                                    !selected.is_empty() && !in_trash,
                                                    |ui| {
                                                        if widgets::button(
                                                            ui,
                                                            widgets::ButtonKind::Danger,
                                                            Some(ICON_DELETE.codepoint),
                                                            &t("selection-trash"),
                                                        )
                                                        .clicked()
                                                        {
                                                            let paths = visible_notes
                                                                .iter()
                                                                .filter(|n| {
                                                                    selected
                                                                        .contains(&n.frontmatter.id)
                                                                })
                                                                .map(|n| n.path.clone())
                                                                .collect();
                                                            actions.push(GridAction::BatchTrash(
                                                                paths,
                                                            ));
                                                        }
                                                        if widgets::secondary_button(
                                                            ui,
                                                            Some(ICON_INVENTORY_2.codepoint),
                                                            &t("selection-archive"),
                                                        )
                                                        .clicked()
                                                        {
                                                            actions.push(GridAction::BatchArchive(
                                                                selected.iter().copied().collect(),
                                                            ));
                                                        }
                                                    },
                                                );
                                            },
                                        );
                                    });
                                });
                        }

                        // ── Search summary ──
                        if !search.is_empty() {
                            ui.add_space(theme::SPACE_S);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(tr.t(
                                        "grid-search-results",
                                        &[("query", &search), ("count", &total.to_string())],
                                    ))
                                    .size(theme::TEXT_SM)
                                    .color(p.text_dim),
                                );
                                if widgets::ghost_button(
                                    ui,
                                    Some(ICON_CLOSE.codepoint),
                                    &t("grid-clear-search"),
                                )
                                .clicked()
                                {
                                    actions.push(GridAction::ClearSearch);
                                }
                            });
                        }

                        ui.add_space(theme::SPACE_L);

                        // ── Empty states ──
                        if total == 0 && semantic_hits.is_empty() {
                            empty_grid(ui, tr, &filter, &search, vault_empty, &mut actions);
                            return;
                        }

                        // ── Masonry grid ──
                        if total > 0 {
                            let gap = theme::GRID_GAP;
                            let columns = (((content_w + gap) / (CARD_WIDTH + gap)).floor()
                                as usize)
                                .clamp(1, MAX_COLUMNS);
                            let col_w = (content_w - gap * (columns as f32 - 1.0)) / columns as f32;
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::new(gap, 0.0);
                                for col in 0..columns {
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(col_w, 0.0),
                                        Layout::top_down(Align::Min),
                                        |ui| {
                                            ui.set_width(col_w);
                                            ui.spacing_mut().item_spacing = Vec2::new(0.0, gap);
                                            for (i, note) in visible_notes.iter().enumerate() {
                                                if i % columns == col {
                                                    note_card(
                                                        ui,
                                                        tr,
                                                        note,
                                                        derived.canvas_previews.get(&note.path),
                                                        *selection_mode,
                                                        selected,
                                                        in_trash,
                                                        now,
                                                        &mut actions,
                                                    );
                                                }
                                            }
                                            let offset = visible_notes.len();
                                            for (i, path) in visible_files.iter().enumerate() {
                                                if (i + offset) % columns == col {
                                                    file_card(
                                                        ui,
                                                        tr,
                                                        path,
                                                        derived.file_sizes.get(*path).copied(),
                                                        &mut actions,
                                                    );
                                                }
                                            }
                                        },
                                    );
                                }
                            });
                        }

                        // ── Semantic ("also relevant") results ──
                        if !semantic_hits.is_empty() {
                            ui.add_space(theme::SPACE_XL);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(ICON_AUTO_AWESOME.codepoint)
                                        .size(16.0)
                                        .color(p.accent),
                                );
                                ui.label(
                                    RichText::new(t("grid-semantic-title"))
                                        .font(theme::semibold(theme::TEXT_BODY))
                                        .color(p.text),
                                );
                                ui.label(
                                    RichText::new(t("grid-semantic-hint"))
                                        .size(theme::TEXT_XS)
                                        .color(p.text_faint),
                                );
                            });
                            ui.add_space(theme::SPACE_S);
                            for hit in semantic_hits {
                                semantic_row(ui, tr, vault, hit, &mut actions);
                            }
                        }
                        ui.add_space(theme::SPACE_XL * 2.0);
                    });
                });
            });

        for action in actions {
            self.apply_grid_action(action);
        }
    }

    fn apply_grid_action(&mut self, action: GridAction) {
        match action {
            GridAction::Open(path) => self.open_file_by_path(path),
            GridAction::TogglePin(id) => {
                self.mutate_note(id, |n| n.frontmatter.pinned = !n.frontmatter.pinned)
            }
            GridAction::SetColor(id, color) => {
                self.mutate_note(id, |n| n.frontmatter.color = color)
            }
            GridAction::ToggleArchived(id) => {
                let was_archived = self
                    .vault
                    .as_ref()
                    .and_then(|v| v.notes.iter().find(|n| n.frontmatter.id == id))
                    .is_some_and(|n| n.frontmatter.archived);
                self.mutate_note(id, |n| n.frontmatter.archived = !was_archived);
                let key = if was_archived {
                    "toast-unarchived"
                } else {
                    "toast-archived"
                };
                let msg = self.t(key);
                let undo = self.t("toast-undo");
                self.toasts.push_with_action(
                    ToastKind::Success,
                    msg,
                    undo,
                    ToastAction::ToggleArchived(id),
                );
            }
            GridAction::Trash(path) => self.trash_note(&path),
            GridAction::Restore(path) => self.restore_note(&path, None),
            GridAction::ConfirmDelete(id) => self.confirm_delete = Some(id),
            GridAction::BatchArchive(ids) => {
                let count = ids.len();
                for id in ids {
                    self.mutate_note(id, |n| n.frontmatter.archived = true);
                }
                self.selected.clear();
                self.selection_mode = false;
                self.toast(
                    ToastKind::Success,
                    "toast-batch-archived",
                    &[("count", &count.to_string())],
                );
            }
            GridAction::BatchTrash(paths) => {
                for path in paths {
                    self.trash_note(&path);
                }
                self.selected.clear();
                self.selection_mode = false;
            }
            GridAction::OpenHit(hit) => {
                self.open_chunk_source(hit.chunk.file_path, hit.chunk.page_num);
            }
            GridAction::TrashFile(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.trash_path(&path, false, &name);
            }
            GridAction::NewNote => self.create_note(None, false),
            GridAction::ImportPdf => self.import_pdf_dialog(),
            GridAction::ClearSearch => {
                self.search_text.clear();
                self.search_results.clear();
            }
            GridAction::ShowAll => self.doc_filter = SidebarDocFilter::All,
            GridAction::EmptyTrash => self.confirm_empty_trash = true,
        }
    }
}

fn empty_grid(
    ui: &mut egui::Ui,
    tr: &LocaleManager,
    filter: &SidebarDocFilter,
    search: &str,
    vault_empty: bool,
    actions: &mut Vec<GridAction>,
) {
    let t = |key: &str| tr.t(key, &[]);
    let p = pal();
    let (icon, title, body, action) = if !search.is_empty() {
        (
            ICON_SEARCH.codepoint,
            tr.t("empty-search-title", &[("query", search)]),
            t("empty-search-body"),
            Some((
                ICON_CLOSE.codepoint,
                t("grid-clear-search"),
                GridAction::ClearSearch,
            )),
        )
    } else if vault_empty {
        (
            ICON_NOTE_ADD.codepoint,
            t("empty-vault-title"),
            t("empty-vault-body"),
            Some((ICON_NOTE_ADD.codepoint, t("notes-new"), GridAction::NewNote)),
        )
    } else {
        match filter {
            SidebarDocFilter::Trashed => (
                ICON_DELETE.codepoint,
                t("empty-trash-title"),
                t("empty-trash-body"),
                None,
            ),
            SidebarDocFilter::Archived => (
                ICON_INVENTORY_2.codepoint,
                t("empty-archive-title"),
                t("empty-archive-body"),
                None,
            ),
            SidebarDocFilter::PdfsOnly => (
                ICON_PICTURE_AS_PDF.codepoint,
                t("empty-pdf-title"),
                t("empty-pdf-body"),
                Some((
                    ICON_UPLOAD_FILE.codepoint,
                    t("pdf-import"),
                    GridAction::ImportPdf,
                )),
            ),
            _ => (
                ICON_DESCRIPTION.codepoint,
                t("empty-filter-title"),
                t("empty-filter-body"),
                Some(("", t("grid-show-all"), GridAction::ShowAll)),
            ),
        }
    };
    let clicked = widgets::empty_state(
        ui,
        icon,
        &title,
        &body,
        action.as_ref().map(|(i, l, _)| (*i, l.as_str())),
    );
    if clicked && let Some((_, _, a)) = action {
        actions.push(a);
    }
    if vault_empty && search.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(theme::SPACE_XS);
            if widgets::ghost_button(ui, Some(ICON_UPLOAD_FILE.codepoint), &t("pdf-import"))
                .clicked()
            {
                actions.push(GridAction::ImportPdf);
            }
            ui.add_space(theme::SPACE_S);
            ui.label(
                RichText::new(t("empty-vault-tip"))
                    .size(theme::TEXT_XS)
                    .color(p.text_faint),
            );
        });
    }
}

fn info_banner(ui: &mut egui::Ui, text: &str) {
    let p = pal();
    egui::Frame::NONE
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::RADIUS_MD)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(theme::TEXT_SM).color(p.text_dim));
        });
}

/// Shared chrome for grid cards. The whole-card click target is registered
/// *before* the content (using last frame's rect) so inner buttons still
/// win clicks. Returns the background response, if known yet.
fn card_shell(
    ui: &mut egui::Ui,
    id: Id,
    fill: egui::Color32,
    highlighted: bool,
    add_contents: impl FnOnce(&mut egui::Ui, bool),
) -> (Option<egui::Response>, bool) {
    let p = pal();
    let prev_rect: Option<Rect> = ui.ctx().data(|d| d.get_temp(id));
    let bg = prev_rect.map(|r| ui.interact(r, id.with("bg"), Sense::click()));
    let bg_popup_id = id.with("bg").with("popup");
    let more_popup_id = id.with("more").with("popup");
    let palette_popup_id = id.with("palette").with("popup");
    let menu_open = egui::Popup::is_id_open(ui.ctx(), bg_popup_id)
        || egui::Popup::is_id_open(ui.ctx(), more_popup_id)
        || egui::Popup::is_id_open(ui.ctx(), palette_popup_id);
    let hovered = prev_rect.is_some_and(|r| ui.rect_contains_pointer(r)) || menu_open;

    let stroke_color = if highlighted {
        p.accent
    } else if hovered {
        p.border_strong
    } else {
        p.border
    };
    let resp = theme::card_frame()
        .fill(fill)
        .stroke(Stroke::new(
            if highlighted { 2.0 } else { 1.0 },
            stroke_color,
        ))
        .show(ui, |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            ui.set_width(ui.available_width());
            add_contents(ui, hovered);
        })
        .response;
    ui.ctx().data_mut(|d| d.insert_temp(id, resp.rect));
    let bg_resp = bg.map(|r| r.on_hover_cursor(egui::CursorIcon::PointingHand));
    if hovered && !menu_open {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (bg_resp, menu_open)
}

#[allow(clippy::too_many_arguments)]
fn note_card(
    ui: &mut egui::Ui,
    tr: &LocaleManager,
    note: &Note,
    canvas_preview: Option<&super::CanvasPreview>,
    selection_mode: bool,
    selected: &mut HashSet<Uuid>,
    in_trash: bool,
    now: DateTime<Utc>,
    actions: &mut Vec<GridAction>,
) {
    let t = |key: &str| tr.t(key, &[]);
    let p = pal();
    let fm = &note.frontmatter;
    let id = fm.id;
    let is_selected = selected.contains(&id);
    let fill = theme::note_tint(fm.color.as_deref()).unwrap_or(p.card);
    let card_id = Id::new(("note_card", id));

    let (bg, menu_open) = card_shell(ui, card_id, fill, is_selected, |ui, hovered| {
        // Title row
        ui.horizontal_top(|ui| {
            let (icon, color) = if note.is_canvas() {
                (ICON_DRAW.codepoint, p.canvas_icon)
            } else {
                (ICON_DESCRIPTION.codepoint, p.note_icon)
            };
            ui.label(RichText::new(icon).size(16.0).color(color));
            ui.add_space(2.0);
            let right_w = if selection_mode || fm.pinned {
                24.0
            } else {
                0.0
            };
            let title = if fm.title.trim().is_empty() {
                t("editor-untitled")
            } else {
                fm.title.clone()
            };
            let mut job = egui::text::LayoutJob::single_section(
                title,
                egui::TextFormat {
                    font_id: theme::semibold(theme::TEXT_BODY + 1.0),
                    color: p.text,
                    ..Default::default()
                },
            );
            job.wrap.max_width = (ui.available_width() - right_w).max(40.0);
            job.wrap.max_rows = 2;
            job.wrap.overflow_character = Some('…');
            ui.label(ui.painter().layout_job(job));
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if selection_mode {
                    let icon = if is_selected {
                        ICON_CHECK_CIRCLE.codepoint
                    } else {
                        ICON_CIRCLE.codepoint
                    };
                    ui.label(RichText::new(icon).size(18.0).color(if is_selected {
                        p.accent
                    } else {
                        p.text_faint
                    }));
                } else if fm.pinned {
                    ui.label(
                        RichText::new(ICON_KEEP.codepoint)
                            .size(15.0)
                            .color(p.warning),
                    )
                    .on_hover_text(t("notes-pinned"));
                }
            });
        });

        // Snippet
        let snippet = match canvas_preview {
            Some(preview) => {
                // Diagram thumbnail (§Fase 3.8): a small vector sketch of
                // the canvas, then the element summary.
                if !preview.thumb.is_empty() {
                    let (rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), CANVAS_THUMB_HEIGHT),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect(
                        rect,
                        theme::RADIUS_SM,
                        if p.is_dark { p.bg } else { p.surface },
                        egui::Stroke::new(1.0, p.border),
                        egui::StrokeKind::Inside,
                    );
                    preview.thumb.paint(ui.painter(), rect, p.is_dark);
                    ui.add_space(4.0);
                }
                ui.label(
                    RichText::new(&preview.summary)
                        .size(theme::TEXT_XS)
                        .color(p.canvas_icon),
                );
                preview.snippet.clone()
            }
            None => query::snippet(&note.body, 180),
        };
        if !snippet.trim().is_empty() {
            ui.add_space(2.0);
            let mut job = egui::text::LayoutJob::single_section(
                snippet,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(theme::TEXT_SM),
                    color: p.text_dim,
                    line_height: Some(19.0),
                    ..Default::default()
                },
            );
            job.wrap.max_width = ui.available_width();
            job.wrap.max_rows = 5;
            job.wrap.overflow_character = Some('…');
            ui.label(ui.painter().layout_job(job));
        }

        // Checklist progress
        if let Some((done, total)) = note.checklist_progress() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new((ui.available_width() - 44.0).max(20.0), 4.0),
                    Sense::hover(),
                );
                ui.painter().rect_filled(rect, 2.0, p.hover);
                let frac = done as f32 / total.max(1) as f32;
                let filled =
                    Rect::from_min_size(rect.min, Vec2::new(rect.width() * frac, rect.height()));
                ui.painter().rect_filled(filled, 2.0, p.success);
                ui.label(
                    RichText::new(format!("{done}/{total}"))
                        .size(theme::TEXT_XS)
                        .color(p.text_faint),
                );
            });
        }

        // Tags (frontmatter + inline `#tag`s)
        let tags = note.effective_tags();
        if !tags.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for tag in &tags {
                    theme::tag_chip_frame(theme::tag_color(tag)).show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("#{tag}"))
                                .size(theme::TEXT_XS)
                                .color(p.text),
                        );
                    });
                }
            });
        }

        // Footer: modified time + actions
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.set_height(26.0);
            ui.label(
                RichText::new(relative_time(tr, fm.modified, now))
                    .size(theme::TEXT_XS)
                    .color(p.text_faint),
            )
            .on_hover_text(
                fm.modified
                    .with_timezone(&chrono::Local)
                    .format("%d %B %Y, %H:%M")
                    .to_string(),
            );
            if selection_mode {
                return;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if in_trash {
                    if widgets::icon_button_sized(
                        ui,
                        ICON_DELETE_FOREVER.codepoint,
                        &t("card-delete-permanent"),
                        false,
                        26.0,
                        16.0,
                    )
                    .clicked()
                    {
                        actions.push(GridAction::ConfirmDelete(id));
                    }
                    if widgets::icon_button_sized(
                        ui,
                        ICON_RESTORE_FROM_TRASH.codepoint,
                        &t("card-restore"),
                        false,
                        26.0,
                        16.0,
                    )
                    .clicked()
                    {
                        actions.push(GridAction::Restore(note.path.clone()));
                    }
                    return;
                }
                if !hovered {
                    return;
                }
                let more = widgets::icon_button_sized_id(
                    ui,
                    card_id.with("more"),
                    ICON_MORE_HORIZ.codepoint,
                    &t("sidebar-more-actions"),
                    false,
                    26.0,
                    16.0,
                );
                egui::Popup::menu(&more)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| card_menu(ui, tr, note, actions));
                let palette = widgets::icon_button_sized_id(
                    ui,
                    card_id.with("palette"),
                    ICON_PALETTE.codepoint,
                    &t("card-color"),
                    false,
                    26.0,
                    16.0,
                );
                egui::Popup::menu(&palette)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| color_menu(ui, tr, note, actions));
                let (pin_icon, pin_tip) = if fm.pinned {
                    (ICON_KEEP.codepoint, t("notes-unpin"))
                } else {
                    (ICON_PUSH_PIN.codepoint, t("notes-pin"))
                };
                if widgets::icon_button_sized(ui, pin_icon, &pin_tip, fm.pinned, 26.0, 16.0)
                    .clicked()
                {
                    actions.push(GridAction::TogglePin(id));
                }
            });
        });
    });

    let Some(bg) = bg else { return };
    if !menu_open && bg.clicked() {
        if selection_mode {
            if !selected.remove(&id) {
                selected.insert(id);
            }
        } else {
            actions.push(GridAction::Open(note.path.clone()));
        }
    }
    if !selection_mode {
        egui::Popup::context_menu(&bg)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                if in_trash {
                    ui.set_min_width(220.0);
                    if widgets::menu_item(
                        ui,
                        ICON_RESTORE_FROM_TRASH.codepoint,
                        &t("card-restore"),
                        None,
                    )
                    .clicked()
                    {
                        actions.push(GridAction::Restore(note.path.clone()));
                        ui.close();
                    }
                    if widgets::menu_item_colored(
                        ui,
                        ICON_DELETE_FOREVER.codepoint,
                        &t("card-delete-permanent"),
                        None,
                        p.danger,
                    )
                    .clicked()
                    {
                        actions.push(GridAction::ConfirmDelete(id));
                        ui.close();
                    }
                } else {
                    card_menu(ui, tr, note, actions);
                }
            });
    }
}

fn card_menu(ui: &mut egui::Ui, tr: &LocaleManager, note: &Note, actions: &mut Vec<GridAction>) {
    let t = |key: &str| tr.t(key, &[]);
    let p = pal();
    let fm = &note.frontmatter;
    ui.set_min_width(230.0);
    if widgets::menu_item(ui, ICON_OPEN_IN_NEW.codepoint, &t("sidebar-open"), None).clicked() {
        actions.push(GridAction::Open(note.path.clone()));
        ui.close();
    }
    let (pin_icon, pin_label) = if fm.pinned {
        (ICON_KEEP.codepoint, t("notes-unpin"))
    } else {
        (ICON_PUSH_PIN.codepoint, t("notes-pin"))
    };
    if widgets::menu_item(ui, pin_icon, &pin_label, None).clicked() {
        actions.push(GridAction::TogglePin(fm.id));
        ui.close();
    }
    let archive_label = if fm.archived {
        t("card-unarchive")
    } else {
        t("card-archive")
    };
    if widgets::menu_item(ui, ICON_INVENTORY_2.codepoint, &archive_label, None).clicked() {
        actions.push(GridAction::ToggleArchived(fm.id));
        ui.close();
    }
    ui.separator();
    color_menu(ui, tr, note, actions);
    ui.separator();
    if widgets::menu_item_colored(ui, ICON_DELETE.codepoint, &t("card-trash"), None, p.danger)
        .clicked()
    {
        actions.push(GridAction::Trash(note.path.clone()));
        ui.close();
    }
}

fn color_menu(ui: &mut egui::Ui, tr: &LocaleManager, note: &Note, actions: &mut Vec<GridAction>) {
    let p = pal();
    widgets::section_header(ui, &tr.t("card-color", &[]));
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(220.0);
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let current = note.frontmatter.color.as_deref();

        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.painter().circle(
            rect.center(),
            9.0,
            p.card,
            Stroke::new(1.0, p.border_strong),
        );
        ui.painter().line_segment(
            [
                rect.center() + Vec2::new(-5.0, 5.0),
                rect.center() + Vec2::new(5.0, -5.0),
            ],
            Stroke::new(1.0, p.text_faint),
        );
        if current.is_none() {
            ui.painter()
                .circle_stroke(rect.center(), 11.5, Stroke::new(1.5, p.accent));
        } else if resp.hovered() {
            ui.painter()
                .circle_stroke(rect.center(), 11.5, Stroke::new(1.0, p.border_strong));
        }
        if resp.on_hover_text(tr.t("card-color-none", &[])).clicked() {
            actions.push(GridAction::SetColor(note.frontmatter.id, None));
            ui.close();
        }
        for (name, color) in theme::PALETTE_SOLID {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.painter().circle_filled(rect.center(), 9.0, *color);
            if current == Some(*name) {
                ui.painter()
                    .circle_stroke(rect.center(), 11.5, Stroke::new(1.5, p.accent));
            } else if resp.hovered() {
                ui.painter()
                    .circle_stroke(rect.center(), 11.5, Stroke::new(1.0, p.border_strong));
            }
            if resp.clicked() {
                actions.push(GridAction::SetColor(
                    note.frontmatter.id,
                    Some((*name).to_string()),
                ));
                ui.close();
            }
        }
    });
}

/// Card for a non-note vault file: an imported PDF or a CSV/XLSX sheet.
fn file_card(
    ui: &mut egui::Ui,
    tr: &LocaleManager,
    path: &Path,
    size: Option<u64>,
    actions: &mut Vec<GridAction>,
) {
    let t = |key: &str| tr.t(key, &[]);
    let p = pal();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Document".to_string());
    let (icon, icon_color, kind) = if crate::sheet::is_sheet_path(path) {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        (ICON_TABLE_CHART.codepoint, p.sheet_icon, ext)
    } else {
        (ICON_PICTURE_AS_PDF.codepoint, p.pdf_icon, "PDF".to_string())
    };
    let card_id = Id::new(("file_card", path));

    let (bg, menu_open) = card_shell(ui, card_id, p.card, false, |ui, hovered| {
        ui.horizontal_top(|ui| {
            ui.label(RichText::new(icon).size(16.0).color(icon_color));
            ui.add_space(2.0);
            let mut job = egui::text::LayoutJob::single_section(
                name.clone(),
                egui::TextFormat {
                    font_id: theme::semibold(theme::TEXT_BODY + 1.0),
                    color: p.text,
                    ..Default::default()
                },
            );
            job.wrap.max_width = ui.available_width();
            job.wrap.max_rows = 2;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            ui.label(ui.painter().layout_job(job));
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.set_height(26.0);
            let size_text = match size {
                Some(bytes) if bytes >= 1_048_576 => {
                    format!("{kind} · {:.1} MB", bytes as f64 / 1_048_576.0)
                }
                Some(bytes) => format!("{kind} · {:.0} KB", (bytes as f64 / 1024.0).max(1.0)),
                None => kind.clone(),
            };
            ui.label(
                RichText::new(size_text)
                    .size(theme::TEXT_XS)
                    .color(p.text_faint),
            );
            if hovered {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button_sized(
                        ui,
                        ICON_DELETE.codepoint,
                        &t("card-trash"),
                        false,
                        26.0,
                        16.0,
                    )
                    .clicked()
                    {
                        actions.push(GridAction::TrashFile(path.to_path_buf()));
                    }
                });
            }
        });
    });

    let Some(bg) = bg else { return };
    if !menu_open && bg.clicked() {
        actions.push(GridAction::Open(path.to_path_buf()));
    }
    egui::Popup::context_menu(&bg)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(200.0);
            if widgets::menu_item(ui, ICON_OPEN_IN_NEW.codepoint, &t("sidebar-open"), None).clicked() {
                actions.push(GridAction::Open(path.to_path_buf()));
                ui.close();
            }
            if widgets::menu_item_colored(ui, ICON_DELETE.codepoint, &t("card-trash"), None, p.danger)
                .clicked()
            {
                actions.push(GridAction::TrashFile(path.to_path_buf()));
                ui.close();
            }
        });
}

fn semantic_row(
    ui: &mut egui::Ui,
    tr: &LocaleManager,
    vault: &Vault,
    hit: &SearchHit,
    actions: &mut Vec<GridAction>,
) {
    let p = pal();
    let chunk = &hit.chunk;
    let file_name = chunk
        .file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let (icon, color, title) = match chunk.page_num {
        Some(row) if crate::sheet::is_sheet_path(&chunk.file_path) => (
            ICON_TABLE_CHART.codepoint,
            p.sheet_icon,
            tr.t(
                "chat-citation-row",
                &[("name", &file_name), ("row", &row.to_string())],
            ),
        ),
        Some(page) => (
            ICON_PICTURE_AS_PDF.codepoint,
            p.pdf_icon,
            tr.t(
                "chat-citation-page",
                &[("name", &file_name), ("page", &page.to_string())],
            ),
        ),
        None => (
            ICON_DESCRIPTION.codepoint,
            p.note_icon,
            vault
                .notes
                .iter()
                .find(|n| n.frontmatter.id == chunk.doc_id)
                .map(|n| n.frontmatter.title.clone())
                .unwrap_or_else(|| chunk.file_path.display().to_string()),
        ),
    };
    let resp = egui::Frame::NONE
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::RADIUS_MD)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(16.0).color(color));
                ui.add_space(2.0);
                ui.label(
                    RichText::new(title)
                        .font(theme::semibold(theme::TEXT_SM + 0.5))
                        .color(p.text),
                );
                let percent = hit.score.map(|s| format!("{:.0}", s * 100.0));
                let badge = match (hit.kind, percent) {
                    (MatchKind::Both, Some(pct)) => tr.t("grid-match-both", &[("percent", &pct)]),
                    (MatchKind::Semantic, Some(pct)) => {
                        tr.t("grid-semantic-match", &[("percent", &pct)])
                    }
                    _ => tr.t("grid-match-keyword", &[]),
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(badge).size(theme::TEXT_XS).color(p.text_faint));
                });
            });
            match &hit.snippet {
                Some(snippet) => {
                    ui.label(highlighted_snippet(snippet, p.text_dim, p.accent));
                }
                None => {
                    ui.label(
                        RichText::new(query::snippet(&chunk.text_content, 200))
                            .size(theme::TEXT_SM)
                            .color(p.text_dim),
                    );
                }
            }
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        actions.push(GridAction::OpenHit(hit.clone()));
    }
    ui.add_space(6.0);
}

/// Lays out an FTS snippet with the matched terms (between
/// `HIGHLIGHT_START`/`HIGHLIGHT_END`) emphasized.
fn highlighted_snippet(snippet: &str, text: egui::Color32, accent: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let normal = egui::TextFormat {
        font_id: egui::FontId::proportional(theme::TEXT_SM),
        color: text,
        ..Default::default()
    };
    let strong = egui::TextFormat {
        font_id: theme::semibold(theme::TEXT_SM),
        color: accent,
        ..Default::default()
    };
    let flat = snippet.replace(['\n', '\r'], " ");
    let mut highlighted = false;
    for part in flat.split([HIGHLIGHT_START, HIGHLIGHT_END]) {
        if !part.is_empty() {
            job.append(part, 0.0, if highlighted { strong.clone() } else { normal.clone() });
        }
        highlighted = !highlighted;
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn highlighted_snippet_emphasizes_marked_terms() {
        let snippet = format!("resep {HIGHLIGHT_START}nasi{HIGHLIGHT_END} goreng");
        let job = highlighted_snippet(&snippet, egui::Color32::GRAY, egui::Color32::RED);
        assert_eq!(job.text, "resep nasi goreng");
        assert_eq!(job.sections.len(), 3);
        assert_eq!(job.sections[1].format.color, egui::Color32::RED);
    }

    fn locales() -> LocaleManager {
        LocaleManager::load(Path::new("/nonexistent-dir-uses-embedded"))
    }

    #[test]
    fn relative_time_buckets() {
        let tr = locales();
        let now = Utc::now();
        assert_eq!(relative_time(&tr, now, now), tr.t("time-just-now", &[]));
        assert!(relative_time(&tr, now - Duration::minutes(5), now).contains('5'));
        assert!(relative_time(&tr, now - Duration::hours(3), now).contains('3'));
        assert!(relative_time(&tr, now - Duration::days(2), now).contains('2'));
        let old = now - Duration::days(400);
        assert_eq!(
            relative_time(&tr, old, now),
            old.format("%d %b %Y").to_string()
        );
        // A timestamp in the future (clock skew) must not go negative.
        assert_eq!(
            relative_time(&tr, now + Duration::hours(1), now),
            tr.t("time-just-now", &[])
        );
    }

    #[test]
    fn card_shell_text_is_not_selectable_and_clicks_card() {
        let ctx = egui::Context::default();
        let card_id = Id::new("test_card");
        let mut clicked = false;

        // Pass 1: measure and lay out card_shell so temp rect is stored
        let mut out1 = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = card_shell(ui, card_id, egui::Color32::WHITE, false, |ui, _| {
                assert!(!ui.style().interaction.selectable_labels);
                ui.label("Clickable Card Title");
            });
        });
        out1.textures_delta.clear();

        let rect = ctx.data(|d| d.get_temp::<Rect>(card_id)).expect("rect stored");
        let click_pos = rect.min + egui::vec2(10.0, 10.0); // directly over the label text

        // Pass 2: register bg widget with the stored rect so it enters prev_pass.widgets
        let mut input2 = egui::RawInput::default();
        input2.events.push(egui::Event::PointerMoved(click_pos));
        let mut out2 = ctx.run_ui(input2, |ui| {
            let (bg, _) = card_shell(ui, card_id, egui::Color32::WHITE, false, |ui, _| {
                ui.label("Clickable Card Title");
            });
            assert!(bg.is_some(), "bg should now be registered");
        });
        out2.textures_delta.clear();

        // Pass 3: pointer button press over the text
        let mut input3 = egui::RawInput::default();
        input3.events.push(egui::Event::PointerButton {
            pos: click_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        let mut out3 = ctx.run_ui(input3, |ui| {
            let _ = card_shell(ui, card_id, egui::Color32::WHITE, false, |ui, _| {
                ui.label("Clickable Card Title");
            });
        });
        out3.textures_delta.clear();

        // Pass 4: pointer button release -> click triggers
        let mut input4 = egui::RawInput::default();
        input4.events.push(egui::Event::PointerButton {
            pos: click_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        let mut out4 = ctx.run_ui(input4, |ui| {
            let (bg, _) = card_shell(ui, card_id, egui::Color32::WHITE, false, |ui, _| {
                ui.label("Clickable Card Title");
            });
            if let Some(bg) = bg
                && bg.clicked()
            {
                clicked = true;
            }
        });

        assert!(clicked, "Card background should be clicked even when clicking over text");
        assert_eq!(out4.platform_output.cursor_icon, egui::CursorIcon::PointingHand);
        out4.textures_delta.clear();
    }
}
