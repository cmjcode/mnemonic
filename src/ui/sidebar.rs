//! Left sidebar: one scrollable column instead of tabs, so everything is
//! one glance away.
//!
//! 1. Vault switcher + a prominent "New note" button (⌘N) with a "+" menu
//!    for canvases, sheets (§3.8), folders and PDF/spreadsheet import.
//! 2. Library: All / Notes / Canvases / PDFs / Archive / Trash, with counts.
//! 3. Folders: the vault's file tree — click to open, drag to move,
//!    right-click or "⋯" for rename / move / trash.
//! 4. Labels: tags with colors and counts.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::{
    Align, Align2, CornerRadius, FontId, Id, Layout, Pos2, Rect, RichText, Sense, Ui, Vec2,
};
use egui_icons::icons::{
    ICON_ADD, ICON_CHECK, ICON_CHEVRON_RIGHT, ICON_CREATE_NEW_FOLDER, ICON_DASHBOARD, ICON_DELETE,
    ICON_DESCRIPTION, ICON_DRAW, ICON_DRIVE_FILE_MOVE, ICON_DRIVE_FILE_RENAME_OUTLINE, ICON_EDIT,
    ICON_EXPAND_MORE, ICON_FOLDER, ICON_FOLDER_OPEN, ICON_INVENTORY_2, ICON_LABEL, ICON_MORE_HORIZ,
    ICON_NOTE_ADD, ICON_PICTURE_AS_PDF, ICON_RESTART_ALT, ICON_TABLE_CHART, ICON_UNFOLD_LESS,
    ICON_UNFOLD_MORE, ICON_UPLOAD_FILE,
};

use crate::i18n::LocaleManager;
use crate::notes::Note;
use crate::ui::theme::{self, pal, tag_color};
use crate::ui::widgets::{self, RowSpec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarDocFilter {
    All,
    NotesOnly,
    WhiteboardsOnly,
    PdfsOnly,
    Archived,
    Trashed,
    Tag(String),
}

/// Node pohon berkas & direktori di dalam vault.
#[derive(Debug, Clone)]
pub struct FileTreeNode {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_canvas: bool,
    pub is_pdf: bool,
    /// CSV/XLSX sheet (§3.8).
    pub is_sheet: bool,
    pub note_title: Option<String>,
    pub children: Vec<FileTreeNode>,
}

impl FileTreeNode {
    /// Bangun hierarki pohon berkas & direktori dari root vault secara rekursif.
    pub fn build(vault_root: &Path, notes: &[Note], pdfs: &[PathBuf]) -> Option<FileTreeNode> {
        if !vault_root.is_dir() {
            return None;
        }
        let root_name = vault_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Vault".to_string());

        let mut root_node = FileTreeNode {
            path: vault_root.to_path_buf(),
            name: root_name,
            is_dir: true,
            is_canvas: false,
            is_pdf: false,
            is_sheet: false,
            note_title: None,
            children: Vec::new(),
        };
        root_node.scan_recursive(vault_root, notes, pdfs);
        Some(root_node)
    }

    fn scan_recursive(&mut self, current_dir: &Path, notes: &[Note], pdfs: &[PathBuf]) {
        let Ok(entries) = std::fs::read_dir(current_dir) else {
            return;
        };

        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Lewati berkas tersembunyi / sistem (.trash, .git, .mnemonic, ...)
            if file_name.starts_with('.') {
                continue;
            }

            let leaf = |is_canvas: bool, is_pdf: bool, note_title: Option<String>| FileTreeNode {
                path: path.clone(),
                name: file_name.clone(),
                is_dir: false,
                is_canvas,
                is_pdf,
                is_sheet: crate::sheet::is_sheet_path(&path),
                note_title,
                children: Vec::new(),
            };

            if path.is_dir() {
                let mut dir_node = FileTreeNode {
                    path: path.clone(),
                    name: file_name.clone(),
                    is_dir: true,
                    is_canvas: false,
                    is_pdf: false,
                    is_sheet: false,
                    note_title: None,
                    children: Vec::new(),
                };
                dir_node.scan_recursive(&path, notes, pdfs);
                dirs.push(dir_node);
            } else if path.is_file() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();

                if ext == "md" {
                    match notes.iter().find(|n| n.path == path) {
                        Some(note) if note.frontmatter.trashed => continue,
                        Some(note) => files.push(leaf(
                            note.is_canvas(),
                            false,
                            Some(note.frontmatter.title.clone()),
                        )),
                        None => files.push(leaf(false, false, None)),
                    }
                } else if ext == "pdf" || pdfs.contains(&path) {
                    files.push(leaf(false, true, None));
                } else if crate::sheet::is_sheet_path(&path) {
                    files.push(leaf(false, false, None));
                }
            }
        }

        // Urutkan: Direktori dulu (abjad), lalu Berkas (abjad)
        dirs.sort_by_key(|a| a.name.to_lowercase());
        files.sort_by_key(|a| a.display_label().to_lowercase());

        self.children.extend(dirs);
        self.children.extend(files);
    }

    /// Label tampilan untuk file atau direktori.
    pub fn display_label(&self) -> String {
        match &self.note_title {
            Some(title) if !self.is_dir && !title.is_empty() => title.clone(),
            _ => self.name.clone(),
        }
    }

    /// Kumpulkan semua direktori di bawah pohon ini untuk modal pemilihan folder.
    pub fn collect_directories(&self, vault_root: &Path, acc: &mut Vec<(PathBuf, String)>) {
        if self.is_dir && self.path != vault_root {
            let relative = self
                .path
                .strip_prefix(vault_root)
                .unwrap_or(&self.path)
                .to_string_lossy()
                .to_string();
            acc.push((self.path.clone(), relative));
        }
        for child in &self.children {
            child.collect_directories(vault_root, acc);
        }
    }

    /// Cek apakah node ini atau anak-anaknya cocok dengan kueri pencarian filter.
    pub fn matches_filter(&self, filter: &str) -> bool {
        if filter.is_empty() {
            return true;
        }
        let filter_lower = filter.to_lowercase();
        self.display_label().to_lowercase().contains(&filter_lower)
            || self.children.iter().any(|c| c.matches_filter(filter))
    }
}

#[derive(Debug, Clone)]
pub enum SidebarEvent {
    SelectFilter(SidebarDocFilter),
    OpenFile(PathBuf),
    ImportPdf,
    /// Copy CSV/XLSX files into the vault (§3.8).
    ImportSheet,
    ManageLabels,
    OpenVaultPicker,
    CreateVault,
    OpenRecentVault(PathBuf),
    RescanVault,

    CreateNote {
        parent_dir: Option<PathBuf>,
    },
    NewCanvas {
        parent_dir: Option<PathBuf>,
    },
    /// New blank CSV sheet (§3.8).
    NewSheet {
        parent_dir: Option<PathBuf>,
    },
    CreateFolder {
        parent_dir: PathBuf,
    },
    RenameItem {
        path: PathBuf,
        is_dir: bool,
        current_name: String,
    },
    MoveItemPrompt {
        path: PathBuf,
        is_dir: bool,
        name: String,
    },
    DeleteItem {
        path: PathBuf,
        is_dir: bool,
        name: String,
    },
    DirectMove {
        src_path: PathBuf,
        dest_dir: PathBuf,
    },
    ToggleFolder(PathBuf),
    ExpandAllFolders,
    CollapseAllFolders,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SidebarCounts {
    pub all: usize,
    pub notes: usize,
    pub canvases: usize,
    pub pdfs: usize,
    pub sheets: usize,
    pub archived: usize,
    pub trashed: usize,
}

pub struct SidebarState<'a> {
    pub vault_root: &'a Path,
    pub vault_name: &'a str,
    pub recent_vaults: &'a [PathBuf],
    pub active_file_path: Option<&'a Path>,
    pub expanded_folders: &'a HashSet<PathBuf>,
    pub file_tree: Option<&'a FileTreeNode>,
    pub current_filter: &'a SidebarDocFilter,
    /// Whether the document grid is showing (filters only highlight then).
    pub home_active: bool,
    pub all_tags: &'a [(String, usize)],
    pub counts: SidebarCounts,
}

pub struct SidebarDrawer;

impl SidebarDrawer {
    /// Renders the collapsible left panel and returns this frame's events.
    pub fn show(
        ui: &mut Ui,
        tr: &LocaleManager,
        state: &SidebarState,
        is_open: &mut bool,
    ) -> Vec<SidebarEvent> {
        let mut events = Vec::new();
        egui::Panel::left("mnemonic_sidebar")
            .resizable(true)
            .default_size(theme::SIDEBAR_WIDTH)
            .size_range(200.0..=440.0)
            .frame(theme::side_panel_frame())
            .show_separator_line(true)
            .show_collapsible(ui, is_open, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                Self::header(ui, tr, state, &mut events);
                ui.add_space(theme::SPACE_S);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        Self::library(ui, tr, state, &mut events);
                        Self::folders(ui, tr, state, &mut events);
                        Self::labels(ui, tr, state, &mut events);
                        ui.add_space(theme::SPACE_XL);
                    });
            });
        events
    }

    fn header(
        ui: &mut Ui,
        tr: &LocaleManager,
        state: &SidebarState,
        events: &mut Vec<SidebarEvent>,
    ) {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();

        // ── Vault switcher ──
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::click());
        if resp.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::RADIUS_MD), p.hover);
        }
        let logo = crate::ui::logo::logo_texture(ui.ctx());
        let logo_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + 6.0, rect.center().y - 12.0),
            Vec2::splat(24.0),
        );
        egui::Image::new((logo.id(), Vec2::splat(24.0)))
            .corner_radius(6.0)
            .paint_at(ui, logo_rect);
        let name_galley = widgets::elided_galley(
            ui,
            state.vault_name,
            theme::semibold(theme::TEXT_BODY),
            p.text,
            width - 64.0,
        );
        ui.painter().galley(
            Pos2::new(
                rect.min.x + 38.0,
                rect.center().y - name_galley.size().y / 2.0,
            ),
            name_galley,
            p.text,
        );
        ui.painter().text(
            Pos2::new(rect.max.x - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            ICON_EXPAND_MORE.codepoint,
            FontId::proportional(18.0),
            p.text_faint,
        );
        let resp = resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(t("sidebar-switch-vault"));
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(260.0);
            widgets::section_header(ui, &t("sidebar-recent-vaults"));
            for vault in state.recent_vaults {
                let name = vault
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| vault.display().to_string());
                let current = vault == state.vault_root;
                if widgets::menu_item(
                    ui,
                    ICON_FOLDER.codepoint,
                    &name,
                    current.then_some(ICON_CHECK.codepoint),
                )
                .on_hover_text(vault.display().to_string())
                .clicked()
                {
                    if !current {
                        events.push(SidebarEvent::OpenRecentVault(vault.clone()));
                    }
                    ui.close();
                }
            }
            ui.separator();
            if widgets::menu_item(
                ui,
                ICON_FOLDER_OPEN.codepoint,
                &t("sidebar-open-other-vault"),
                None,
            )
            .clicked()
            {
                events.push(SidebarEvent::OpenVaultPicker);
                ui.close();
            }
            if widgets::menu_item(
                ui,
                ICON_CREATE_NEW_FOLDER.codepoint,
                &t("welcome-create-vault"),
                None,
            )
            .clicked()
            {
                events.push(SidebarEvent::CreateVault);
                ui.close();
            }
            if widgets::menu_item(ui, ICON_RESTART_ALT.codepoint, &t("sidebar-rescan"), None)
                .clicked()
            {
                events.push(SidebarEvent::RescanVault);
                ui.close();
            }
        });

        ui.add_space(theme::SPACE_S);

        // ── "New note" button + "+" menu for other types ──
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let main_w = (ui.available_width() - theme::CONTROL_HEIGHT - 6.0).max(80.0);
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(main_w, theme::CONTROL_HEIGHT), Sense::click());
            let fill = if resp.hovered() {
                p.accent_hover
            } else {
                p.accent
            };
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::RADIUS_MD), fill);
            ui.painter().text(
                Pos2::new(rect.min.x + 12.0, rect.center().y),
                Align2::LEFT_CENTER,
                ICON_NOTE_ADD.codepoint,
                FontId::proportional(theme::ICON_SIZE - 1.0),
                p.on_accent,
            );
            ui.painter().text(
                Pos2::new(rect.min.x + 35.0, rect.center().y),
                Align2::LEFT_CENTER,
                t("notes-new"),
                FontId::proportional(theme::TEXT_SM + 0.5),
                p.on_accent,
            );
            ui.painter().text(
                Pos2::new(rect.max.x - 10.0, rect.center().y),
                Align2::RIGHT_CENTER,
                "⌘N",
                FontId::proportional(theme::TEXT_XS),
                p.on_accent.gamma_multiply(0.75),
            );
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                events.push(SidebarEvent::CreateNote { parent_dir: None });
            }

            let (more_rect, more) =
                ui.allocate_exact_size(Vec2::splat(theme::CONTROL_HEIGHT), Sense::click());
            ui.painter().rect(
                more_rect,
                CornerRadius::same(theme::RADIUS_MD),
                if more.hovered() { p.hover } else { p.card },
                egui::Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                more_rect.center(),
                Align2::CENTER_CENTER,
                ICON_ADD.codepoint,
                FontId::proportional(theme::ICON_SIZE),
                p.text,
            );
            let more = more
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(t("sidebar-new-other"));
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(220.0);
                if widgets::menu_item(ui, ICON_DRAW.codepoint, &t("sidebar-new-canvas"), None)
                    .clicked()
                {
                    events.push(SidebarEvent::NewCanvas { parent_dir: None });
                    ui.close();
                }
                if widgets::menu_item(ui, ICON_TABLE_CHART.codepoint, &t("sidebar-new-sheet"), None)
                    .clicked()
                {
                    events.push(SidebarEvent::NewSheet { parent_dir: None });
                    ui.close();
                }
                if widgets::menu_item(
                    ui,
                    ICON_CREATE_NEW_FOLDER.codepoint,
                    &t("sidebar-new-folder"),
                    None,
                )
                .clicked()
                {
                    events.push(SidebarEvent::CreateFolder {
                        parent_dir: state.vault_root.to_path_buf(),
                    });
                    ui.close();
                }
                if widgets::menu_item(ui, ICON_UPLOAD_FILE.codepoint, &t("pdf-import"), None)
                    .clicked()
                {
                    events.push(SidebarEvent::ImportPdf);
                    ui.close();
                }
                if widgets::menu_item(ui, ICON_UPLOAD_FILE.codepoint, &t("sheet-import"), None)
                    .clicked()
                {
                    events.push(SidebarEvent::ImportSheet);
                    ui.close();
                }
            });
        });
    }

    fn library(
        ui: &mut Ui,
        tr: &LocaleManager,
        state: &SidebarState,
        events: &mut Vec<SidebarEvent>,
    ) {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let c = state.counts;
        let items = [
            (
                SidebarDocFilter::All,
                ICON_DASHBOARD.codepoint,
                p.text_dim,
                "sidebar-all",
                c.all,
            ),
            (
                SidebarDocFilter::NotesOnly,
                ICON_DESCRIPTION.codepoint,
                p.note_icon,
                "sidebar-notes-only",
                c.notes,
            ),
            (
                SidebarDocFilter::WhiteboardsOnly,
                ICON_DRAW.codepoint,
                p.canvas_icon,
                "sidebar-whiteboards-only",
                c.canvases,
            ),
            (
                SidebarDocFilter::PdfsOnly,
                ICON_PICTURE_AS_PDF.codepoint,
                p.pdf_icon,
                "sidebar-pdfs-only",
                c.pdfs,
            ),
            (
                SidebarDocFilter::Archived,
                ICON_INVENTORY_2.codepoint,
                p.text_dim,
                "sidebar-archived",
                c.archived,
            ),
            (
                SidebarDocFilter::Trashed,
                ICON_DELETE.codepoint,
                p.text_dim,
                "sidebar-trash",
                c.trashed,
            ),
        ];
        widgets::section_header(ui, &t("sidebar-library"));
        for (filter, icon, color, key, count) in items {
            let selected = state.home_active && state.current_filter == &filter;
            let label = t(key);
            let count_text = count.to_string();
            let resp = widgets::list_row(
                ui,
                RowSpec {
                    icon,
                    icon_color: color,
                    label: &label,
                    trailing: (count > 0).then_some(count_text.as_str()),
                    selected,
                    indent: 0.0,
                    reserve_right: 0.0,
                },
            );
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                events.push(SidebarEvent::SelectFilter(filter));
            }
        }
    }

    /// A section label with right-aligned small icon actions.
    fn section_with_actions(ui: &mut Ui, text: &str, actions: impl FnOnce(&mut Ui)) -> Rect {
        let p = pal();
        ui.add_space(theme::SPACE_M);
        ui.horizontal(|ui| {
            ui.set_height(24.0);
            ui.add_space(8.0);
            ui.label(
                RichText::new(text.to_uppercase())
                    .font(theme::semibold(11.0))
                    .color(p.text_faint),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                actions(ui);
            });
        })
        .response
        .rect
    }

    fn folders(
        ui: &mut Ui,
        tr: &LocaleManager,
        state: &SidebarState,
        events: &mut Vec<SidebarEvent>,
    ) {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();

        let header_rect = Self::section_with_actions(ui, &t("sidebar-folders"), |ui| {
            let all_collapsed = state.expanded_folders.is_empty();
            let (icon, tip) = if all_collapsed {
                (ICON_UNFOLD_MORE.codepoint, t("sidebar-expand-all"))
            } else {
                (ICON_UNFOLD_LESS.codepoint, t("sidebar-collapse-all"))
            };
            if widgets::icon_button_sized(ui, icon, &tip, false, 24.0, 15.0).clicked() {
                events.push(if all_collapsed {
                    SidebarEvent::ExpandAllFolders
                } else {
                    SidebarEvent::CollapseAllFolders
                });
            }
            if widgets::icon_button_sized(
                ui,
                ICON_CREATE_NEW_FOLDER.codepoint,
                &t("sidebar-new-folder"),
                false,
                24.0,
                15.0,
            )
            .clicked()
            {
                events.push(SidebarEvent::CreateFolder {
                    parent_dir: state.vault_root.to_path_buf(),
                });
            }
        });

        // The section header doubles as a drop target for "move to vault root".
        let drop = ui.interact(header_rect, Id::new("sidebar_root_drop"), Sense::hover());
        let movable_to_root = |payload: &PathBuf| payload.parent() != Some(state.vault_root);
        if let Some(payload) = drop.dnd_hover_payload::<PathBuf>()
            && movable_to_root(&payload)
        {
            ui.painter().rect_stroke(
                header_rect,
                CornerRadius::same(theme::RADIUS_MD),
                egui::Stroke::new(1.5, p.accent),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(payload) = drop.dnd_release_payload::<PathBuf>()
            && movable_to_root(&payload)
        {
            events.push(SidebarEvent::DirectMove {
                src_path: payload.to_path_buf(),
                dest_dir: state.vault_root.to_path_buf(),
            });
        }
        ui.add_space(2.0);

        match state.file_tree {
            Some(tree) if !tree.children.is_empty() => {
                for child in &tree.children {
                    Self::tree_item(ui, tr, child, 0, state, events);
                }
            }
            _ => {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(theme::SPACE_M);
                    ui.label(
                        RichText::new(t("sidebar-folders-empty"))
                            .size(theme::TEXT_XS)
                            .color(p.text_faint),
                    );
                });
            }
        }
    }

    fn tree_item(
        ui: &mut Ui,
        tr: &LocaleManager,
        node: &FileTreeNode,
        depth: usize,
        state: &SidebarState,
        events: &mut Vec<SidebarEvent>,
    ) {
        let p = pal();
        let t = |key: &str| tr.t(key, &[]);
        let indent = depth as f32 * 14.0;
        let label = node.display_label();
        let more_id = Id::new(("sidebar_row_more", &node.path));
        let menu_open = egui::Popup::is_id_open(ui.ctx(), more_id.with("popup"));
        let expanded = node.is_dir && state.expanded_folders.contains(&node.path);

        let (icon, icon_color) = if node.is_dir {
            (
                if expanded {
                    ICON_FOLDER_OPEN.codepoint
                } else {
                    ICON_FOLDER.codepoint
                },
                p.folder_icon,
            )
        } else if node.is_canvas {
            (ICON_DRAW.codepoint, p.canvas_icon)
        } else if node.is_pdf {
            (ICON_PICTURE_AS_PDF.codepoint, p.pdf_icon)
        } else if node.is_sheet {
            (ICON_TABLE_CHART.codepoint, p.sheet_icon)
        } else {
            (ICON_DESCRIPTION.codepoint, p.note_icon)
        };
        let selected = !node.is_dir && state.active_file_path == Some(node.path.as_path());
        let count_text = node.children.len().to_string();

        let resp = widgets::list_row(
            ui,
            RowSpec {
                icon,
                icon_color,
                label: &label,
                trailing: (node.is_dir && !node.children.is_empty()).then_some(count_text.as_str()),
                selected,
                indent: indent + 16.0,
                reserve_right: if node.is_dir { 60.0 } else { 34.0 },
            },
        );
        let rect = resp.rect;

        if node.is_dir {
            ui.painter().text(
                Pos2::new(rect.min.x + 6.0 + indent, rect.center().y),
                Align2::LEFT_CENTER,
                if expanded {
                    ICON_EXPAND_MORE.codepoint
                } else {
                    ICON_CHEVRON_RIGHT.codepoint
                },
                FontId::proportional(15.0),
                p.text_faint,
            );
        }

        // Drag & drop: every row is a drag source; folders accept drops.
        resp.dnd_set_drag_payload(node.path.clone());
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if node.is_dir {
            let accepts =
                |payload: &PathBuf| *payload != node.path && !node.path.starts_with(payload);
            if let Some(payload) = resp.dnd_hover_payload::<PathBuf>()
                && accepts(&payload)
            {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(theme::RADIUS_MD),
                    egui::Stroke::new(1.5, p.accent),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(payload) = resp.dnd_release_payload::<PathBuf>()
                && accepts(&payload)
            {
                events.push(SidebarEvent::DirectMove {
                    src_path: payload.to_path_buf(),
                    dest_dir: node.path.clone(),
                });
            }
        }

        // Hover actions: "⋯" menu, plus "+" (new note here) for folders.
        if resp.hovered() || menu_open {
            let mut x = rect.max.x - 16.0;
            let more = widgets::row_action(
                ui,
                more_id,
                Pos2::new(x, rect.center().y),
                ICON_MORE_HORIZ.codepoint,
                &t("sidebar-more-actions"),
            );
            egui::Popup::menu(&more).show(|ui| Self::item_menu(ui, tr, node, events));
            if node.is_dir {
                x -= 26.0;
                if widgets::row_action(
                    ui,
                    Id::new(("sidebar_row_add", &node.path)),
                    Pos2::new(x, rect.center().y),
                    ICON_ADD.codepoint,
                    &t("sidebar-new-note-here"),
                )
                .clicked()
                {
                    events.push(SidebarEvent::CreateNote {
                        parent_dir: Some(node.path.clone()),
                    });
                }
            }
        }
        egui::Popup::context_menu(&resp).show(|ui| Self::item_menu(ui, tr, node, events));

        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            events.push(if node.is_dir {
                SidebarEvent::ToggleFolder(node.path.clone())
            } else {
                SidebarEvent::OpenFile(node.path.clone())
            });
        }

        if expanded {
            if node.children.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(indent + 48.0);
                    ui.label(
                        RichText::new(t("sidebar-folder-empty"))
                            .size(theme::TEXT_XS)
                            .italics()
                            .color(p.text_faint),
                    );
                });
            }
            for child in &node.children {
                Self::tree_item(ui, tr, child, depth + 1, state, events);
            }
        }
    }

    fn item_menu(
        ui: &mut Ui,
        tr: &LocaleManager,
        node: &FileTreeNode,
        events: &mut Vec<SidebarEvent>,
    ) {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        ui.set_min_width(230.0);
        if node.is_dir {
            if widgets::menu_item(
                ui,
                ICON_NOTE_ADD.codepoint,
                &t("sidebar-new-note-here"),
                None,
            )
            .clicked()
            {
                events.push(SidebarEvent::CreateNote {
                    parent_dir: Some(node.path.clone()),
                });
                ui.close();
            }
            if widgets::menu_item(ui, ICON_DRAW.codepoint, &t("sidebar-new-canvas-here"), None)
                .clicked()
            {
                events.push(SidebarEvent::NewCanvas {
                    parent_dir: Some(node.path.clone()),
                });
                ui.close();
            }
            if widgets::menu_item(
                ui,
                ICON_TABLE_CHART.codepoint,
                &t("sidebar-new-sheet-here"),
                None,
            )
            .clicked()
            {
                events.push(SidebarEvent::NewSheet {
                    parent_dir: Some(node.path.clone()),
                });
                ui.close();
            }
            if widgets::menu_item(
                ui,
                ICON_CREATE_NEW_FOLDER.codepoint,
                &t("sidebar-new-subfolder"),
                None,
            )
            .clicked()
            {
                events.push(SidebarEvent::CreateFolder {
                    parent_dir: node.path.clone(),
                });
                ui.close();
            }
            ui.separator();
        } else if widgets::menu_item(ui, ICON_EDIT.codepoint, &t("sidebar-open"), None).clicked() {
            events.push(SidebarEvent::OpenFile(node.path.clone()));
            ui.close();
        }

        if widgets::menu_item(
            ui,
            ICON_DRIVE_FILE_RENAME_OUTLINE.codepoint,
            &t("tag-rename"),
            None,
        )
        .clicked()
        {
            events.push(SidebarEvent::RenameItem {
                path: node.path.clone(),
                is_dir: node.is_dir,
                current_name: if node.is_dir {
                    node.name.clone()
                } else {
                    node.display_label()
                },
            });
            ui.close();
        }
        if widgets::menu_item(
            ui,
            ICON_DRIVE_FILE_MOVE.codepoint,
            &t("sidebar-move-to"),
            None,
        )
        .clicked()
        {
            events.push(SidebarEvent::MoveItemPrompt {
                path: node.path.clone(),
                is_dir: node.is_dir,
                name: node.display_label(),
            });
            ui.close();
        }
        ui.separator();
        if widgets::menu_item_colored(ui, ICON_DELETE.codepoint, &t("card-trash"), None, p.danger)
            .clicked()
        {
            events.push(SidebarEvent::DeleteItem {
                path: node.path.clone(),
                is_dir: node.is_dir,
                name: node.display_label(),
            });
            ui.close();
        }
    }

    fn labels(
        ui: &mut Ui,
        tr: &LocaleManager,
        state: &SidebarState,
        events: &mut Vec<SidebarEvent>,
    ) {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        Self::section_with_actions(ui, &t("sidebar-tags"), |ui| {
            if !state.all_tags.is_empty()
                && widgets::icon_button_sized(
                    ui,
                    ICON_EDIT.codepoint,
                    &t("sidebar-manage-tags"),
                    false,
                    24.0,
                    15.0,
                )
                .clicked()
            {
                events.push(SidebarEvent::ManageLabels);
            }
        });
        ui.add_space(2.0);

        if state.all_tags.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(theme::SPACE_M);
                ui.label(
                    RichText::new(t("sidebar-tags-empty"))
                        .size(theme::TEXT_XS)
                        .color(p.text_faint),
                );
            });
            return;
        }

        for (tag, count) in state.all_tags {
            let selected = state.home_active
                && matches!(state.current_filter, SidebarDocFilter::Tag(t) if t.eq_ignore_ascii_case(tag));
            let label = format!("#{tag}");
            let count_text = count.to_string();
            let resp = widgets::list_row(
                ui,
                RowSpec {
                    icon: ICON_LABEL.codepoint,
                    icon_color: tag_color(tag),
                    label: &label,
                    trailing: Some(&count_text),
                    selected,
                    indent: 0.0,
                    reserve_right: 0.0,
                },
            );
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                events.push(SidebarEvent::SelectFilter(SidebarDocFilter::Tag(
                    tag.clone(),
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_file_tree_build_and_collect_dirs() {
        let dir = tempdir().unwrap();
        let sub1 = dir.path().join("Folder A");
        let sub2 = dir.path().join("Folder B");
        let nested = sub1.join("Sub A1");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&sub2).unwrap();

        let note1 = Note::create(dir.path(), "Root Note", "body").unwrap();
        let note2 = Note::create(&sub1, "Nested Note", "body").unwrap();

        let notes = vec![note1, note2];
        let tree = FileTreeNode::build(dir.path(), &notes, &[]).unwrap();
        assert_eq!(tree.children.len(), 3); // Folder A, Folder B, Root Note

        let mut dirs = Vec::new();
        tree.collect_directories(dir.path(), &mut dirs);
        assert_eq!(dirs.len(), 3); // Folder A, Sub A1, Folder B
    }

    #[test]
    fn test_file_tree_hides_dot_folders_like_trash() {
        let dir = tempdir().unwrap();
        let trash = dir.path().join(".trash");
        std::fs::create_dir_all(&trash).unwrap();
        Note::create(&trash, "Gone", "x").unwrap();
        Note::create(dir.path(), "Here", "x").unwrap();

        let tree = FileTreeNode::build(dir.path(), &[], &[]).unwrap();
        assert_eq!(tree.children.len(), 1);
    }

    #[test]
    fn test_file_tree_filter_matching() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("Projects");
        std::fs::create_dir_all(&sub).unwrap();

        let note1 = Note::create(dir.path(), "Daily Reflection", "thoughts").unwrap();
        let note2 = Note::create(&sub, "Architecture Roadmap", "design").unwrap();

        let notes = vec![note1, note2];
        let tree = FileTreeNode::build(dir.path(), &notes, &[]).unwrap();

        assert!(tree.matches_filter("Roadmap"));
        assert!(tree.matches_filter("daily"));
        assert!(tree.matches_filter("Projects"));
        assert!(!tree.matches_filter("NonExistentString12345"));
    }

    #[test]
    fn test_file_tree_reorganize_move_file() {
        let dir = tempdir().unwrap();
        let sub1 = dir.path().join("Folder A");
        let sub2 = dir.path().join("Folder B");
        std::fs::create_dir_all(&sub1).unwrap();
        std::fs::create_dir_all(&sub2).unwrap();

        let note = Note::create(&sub1, "Task List", "tasks").unwrap();
        let target_path = sub2.join(note.path.file_name().unwrap());
        std::fs::rename(&note.path, &target_path).unwrap();

        let notes = vec![Note::load(&target_path).unwrap()];
        let tree = FileTreeNode::build(dir.path(), &notes, &[]).unwrap();

        let folder_b = tree.children.iter().find(|c| c.name == "Folder B").unwrap();
        assert_eq!(folder_b.children.len(), 1);
        assert_eq!(folder_b.children[0].display_label(), "Task List");
    }
}
