//! Slide-over Glass Drawer / Sidebar bergaya Shapr3D / DUCAD.
//!
//! Menampilkan drawer mengambang di sisi kiri untuk:
//! 1. Pohon hierarki berkas & folder (File Tree Explorer) dengan reorganisasi struktur lengkap
//! 2. Navigasi filter dokumen, tag label berwarna, dan perpustakaan dokumen PDF.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::{
    Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use egui_icons::icons::{
    ICON_ADD, ICON_CATEGORY, ICON_CLOSE, ICON_DELETE, ICON_DESCRIPTION, ICON_DRAW, ICON_EDIT,
    ICON_FOLDER, ICON_FOLDER_OPEN, ICON_INVENTORY_2, ICON_NOTE_ADD, ICON_PICTURE_AS_PDF,
    ICON_RESTART_ALT, ICON_UPLOAD,
};

use crate::notes::Note;
use crate::ui::theme::{
    fixed_sidebar_frame, tag_color, ACCENT_BLUE, BG_CARD_DARK, BG_HOVER_DARK, BORDER_SUBTLE,
    ROUNDING_SM, SIDEBAR_WIDTH, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarTab {
    #[default]
    Files,
    Filters,
}

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
    pub note_title: Option<String>,
    pub children: Vec<FileTreeNode>,
}

impl FileTreeNode {
    /// Bangun hierarki pohon berkas & direktori dari root vault secara rekursif.
    pub fn build(vault_root: &Path, notes: &[Note], pdfs: &[PathBuf]) -> Option<FileTreeNode> {
        if !vault_root.exists() || !vault_root.is_dir() {
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
            note_title: None,
            children: Vec::new(),
        };

        root_node.scan_recursive(vault_root, notes, pdfs);
        Some(root_node)
    }

    fn scan_recursive(&mut self, current_dir: &Path, notes: &[Note], pdfs: &[PathBuf]) {
        let entries = match std::fs::read_dir(current_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Lewati berkas tersembunyi / sistem
            if file_name.starts_with('.') || file_name == ".trash" || file_name == ".git" {
                continue;
            }

            if path.is_dir() {
                let mut dir_node = FileTreeNode {
                    path: path.clone(),
                    name: file_name,
                    is_dir: true,
                    is_canvas: false,
                    is_pdf: false,
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
                    // Cek apakah ada di note aktif (bukan trashed)
                    let matched_note = notes.iter().find(|n| n.path == path);
                    if let Some(note) = matched_note {
                        if note.frontmatter.trashed {
                            continue; // Note di tong sampah tidak ditampilkan di pohon utama
                        }
                        files.push(FileTreeNode {
                            path: path.clone(),
                            name: file_name,
                            is_dir: false,
                            is_canvas: note.is_canvas(),
                            is_pdf: false,
                            note_title: Some(note.frontmatter.title.clone()),
                            children: Vec::new(),
                        });
                    } else {
                        // File .md biasa
                        files.push(FileTreeNode {
                            path: path.clone(),
                            name: file_name,
                            is_dir: false,
                            is_canvas: false,
                            is_pdf: false,
                            note_title: None,
                            children: Vec::new(),
                        });
                    }
                } else if ext == "pdf" || pdfs.contains(&path) {
                    files.push(FileTreeNode {
                        path: path.clone(),
                        name: file_name,
                        is_dir: false,
                        is_canvas: false,
                        is_pdf: true,
                        note_title: None,
                        children: Vec::new(),
                    });
                }
            }
        }

        // Urutkan: Direktori dulu (abjad), lalu Berkas (abjad)
        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| {
            let name_a = a.display_label();
            let name_b = b.display_label();
            name_a.to_lowercase().cmp(&name_b.to_lowercase())
        });

        self.children.extend(dirs);
        self.children.extend(files);
    }

    /// Label tampilan untuk file atau direktori.
    pub fn display_label(&self) -> String {
        if self.is_dir {
            self.name.clone()
        } else if let Some(title) = &self.note_title {
            if !title.is_empty() {
                title.clone()
            } else {
                self.name.clone()
            }
        } else {
            self.name.clone()
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
        if self.display_label().to_lowercase().contains(&filter_lower) {
            return true;
        }
        self.children.iter().any(|c| c.matches_filter(filter))
    }
}

#[derive(Debug, Clone)]
pub enum SidebarEvent {
    // Navigasi Filter & Buka Berkas
    SelectFilter(SidebarDocFilter),
    OpenFile(PathBuf),
    OpenPdf(PathBuf),
    ImportPdf,
    ManageLabels,
    CloseSidebar,
    OpenVaultPicker,
    SetTab(SidebarTab),
    SearchFilterChanged(String),

    // Reorganisasi Berkas & Folder
    CreateNote { parent_dir: Option<PathBuf> },
    NewCanvas { parent_dir: Option<PathBuf> },
    CreateFolder { parent_dir: PathBuf },
    RenameItem { path: PathBuf, is_dir: bool, current_name: String },
    MoveItemPrompt { path: PathBuf, is_dir: bool, name: String },
    DeleteItem { path: PathBuf, is_dir: bool, name: String },
    DirectMove { src_path: PathBuf, dest_dir: PathBuf },
    ToggleFolder(PathBuf),
    ExpandAllFolders,
    CollapseAllFolders,
    RescanVault,
}

pub struct SidebarState {
    pub is_open: bool,
    pub active_tab: SidebarTab,
    pub vault_root: Option<PathBuf>,
    pub vault_name: String,
    pub active_file_path: Option<PathBuf>,
    pub expanded_folders: HashSet<PathBuf>,
    pub file_tree: Option<FileTreeNode>,
    pub search_filter: String,
    pub current_filter: SidebarDocFilter,
    pub all_tags: Vec<(String, usize)>,
    pub pdf_documents: Vec<PathBuf>,
    pub total_notes: usize,
    pub total_pdfs: usize,
    pub total_whiteboards: usize,
    pub total_archived: usize,
    pub total_trashed: usize,
}

pub struct SidebarDrawer;

impl SidebarDrawer {
    /// Render fixed left sidebar panel. Mengembalikan `Option<SidebarEvent>`.
    pub fn show(ui: &mut egui::Ui, state: &SidebarState) -> Option<SidebarEvent> {
        if !state.is_open {
            return None;
        }

        let mut event = None;

        egui::Panel::left("main_fixed_sidebar")
            .resizable(true)
            .default_size(SIDEBAR_WIDTH)
            .min_size(200.0)
            .max_size(500.0)
            .frame(fixed_sidebar_frame())
            .show(ui, |ui| {
                ui.add_space(6.0);

                // ── Header Drawer (Vault Name & Switcher / Close) ──
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(ICON_FOLDER_OPEN.codepoint)
                            .size(16.0)
                            .color(ACCENT_BLUE),
                    );
                    let vname = if state.vault_name.is_empty() {
                        "Vault".to_string()
                    } else {
                        state.vault_name.clone()
                    };
                    ui.label(
                        RichText::new(vname)
                            .size(13.5)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        let close_btn = egui::Button::new(
                            RichText::new(ICON_CLOSE.codepoint)
                                .size(14.0)
                                .color(TEXT_SECONDARY),
                        )
                        .frame(false);

                        if ui.add(close_btn).on_hover_text("Tutup Sidebar").clicked() {
                            event = Some(SidebarEvent::CloseSidebar);
                        }

                        let rescan_btn = egui::Button::new(
                            RichText::new(ICON_RESTART_ALT.codepoint)
                                .size(13.0)
                                .color(TEXT_MUTED),
                        )
                        .frame(false);

                        if ui
                            .add(rescan_btn)
                            .on_hover_text("Segarkan Berkas (Rescan)")
                            .clicked()
                        {
                            event = Some(SidebarEvent::RescanVault);
                        }
                    });
                });

                ui.add_space(6.0);

                // ── Tab Navigasi Segmented (Berkas vs Filter) ──
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let tab_w = (ui.available_width() - 14.0) / 2.0;

                    // Tab 1: Berkas & Folder
                    let is_files = state.active_tab == SidebarTab::Files;
                    let files_btn = egui::Button::new(
                        RichText::new(format!("{} Berkas", ICON_FOLDER.codepoint))
                            .size(11.5)
                            .color(if is_files { Color32::WHITE } else { TEXT_SECONDARY }),
                    )
                    .fill(if is_files { ACCENT_BLUE } else { BG_CARD_DARK })
                    .corner_radius(CornerRadius::same(ROUNDING_SM));

                    if ui.add_sized(Vec2::new(tab_w, 25.0), files_btn).clicked() {
                        event = Some(SidebarEvent::SetTab(SidebarTab::Files));
                    }

                    // Tab 2: Kategori & Tag
                    let is_filters = state.active_tab == SidebarTab::Filters;
                    let filters_btn = egui::Button::new(
                        RichText::new(format!("{} Kategori", ICON_CATEGORY.codepoint))
                            .size(11.5)
                            .color(if is_filters { Color32::WHITE } else { TEXT_SECONDARY }),
                    )
                    .fill(if is_filters { ACCENT_BLUE } else { BG_CARD_DARK })
                    .corner_radius(CornerRadius::same(ROUNDING_SM));

                    if ui.add_sized(Vec2::new(tab_w, 25.0), filters_btn).clicked() {
                        event = Some(SidebarEvent::SetTab(SidebarTab::Filters));
                    }
                });

                ui.add_space(6.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(6.0);

                // ── KONTEN TAB ──
                match state.active_tab {
                    SidebarTab::Files => {
                        Self::render_files_tab(ui, state, &mut event);
                    }
                    SidebarTab::Filters => {
                        Self::render_filters_tab(ui, state, &mut event);
                    }
                }
            });

        event
    }

    /// Render Tab Berkas & Folder (Pohon Hierarki & Reorganisasi).
    fn render_files_tab(
        ui: &mut Ui,
        state: &SidebarState,
        event: &mut Option<SidebarEvent>,
    ) {
        // Toolbar Cepat Pembuatan & Aksi File Tree
        ui.horizontal(|ui| {
            ui.add_space(6.0);

            // 1. Tambah Catatan Baru di root
            let new_note_btn = egui::Button::new(
                RichText::new(format!("{} Dokumen", ICON_NOTE_ADD.codepoint))
                    .size(11.0)
                    .color(Color32::WHITE),
            )
            .fill(ACCENT_BLUE)
            .corner_radius(CornerRadius::same(ROUNDING_SM));

            if ui
                .add_sized(Vec2::new(76.0, 24.0), new_note_btn)
                .on_hover_text("Buat Catatan Baru")
                .clicked()
            {
                *event = Some(SidebarEvent::CreateNote { parent_dir: None });
            }

            // 2. Tambah Kanvas Baru
            let new_canvas_btn = egui::Button::new(
                RichText::new(format!("{} Kanvas", ICON_DRAW.codepoint))
                    .size(11.0)
                    .color(TEXT_PRIMARY),
            )
            .fill(BG_CARD_DARK)
            .stroke(Stroke::new(0.5, BORDER_SUBTLE))
            .corner_radius(CornerRadius::same(ROUNDING_SM));

            if ui
                .add_sized(Vec2::new(70.0, 24.0), new_canvas_btn)
                .on_hover_text("Buat Kanvas Whiteboard Baru")
                .clicked()
            {
                *event = Some(SidebarEvent::NewCanvas { parent_dir: None });
            }

            // 3. Tambah Folder Baru di root
            let new_folder_btn = egui::Button::new(
                RichText::new(format!("{} Folder", ICON_ADD.codepoint))
                    .size(11.0)
                    .color(TEXT_PRIMARY),
            )
            .fill(BG_CARD_DARK)
            .stroke(Stroke::new(0.5, BORDER_SUBTLE))
            .corner_radius(CornerRadius::same(ROUNDING_SM));

            let root_dir = state
                .vault_root
                .clone()
                .unwrap_or_else(|| PathBuf::from("."));

            if ui
                .add_sized(Vec2::new(66.0, 24.0), new_folder_btn)
                .on_hover_text("Buat Folder Baru di Root")
                .clicked()
            {
                *event = Some(SidebarEvent::CreateFolder {
                    parent_dir: root_dir,
                });
            }

            // 4. Expand / Collapse All
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let toggle_all_btn = egui::Button::new(
                    RichText::new(if state.expanded_folders.is_empty() {
                        "▼"
                    } else {
                        "▲"
                    })
                    .size(10.0)
                    .color(TEXT_MUTED),
                )
                .frame(false);

                if ui
                    .add(toggle_all_btn)
                    .on_hover_text(if state.expanded_folders.is_empty() {
                        "Buka Semua Folder"
                    } else {
                        "Tutup Semua Folder"
                    })
                    .clicked()
                {
                    if state.expanded_folders.is_empty() {
                        *event = Some(SidebarEvent::ExpandAllFolders);
                    } else {
                        *event = Some(SidebarEvent::CollapseAllFolders);
                    }
                }
            });
        });

        ui.add_space(4.0);

        // Filter / Search Bar di dalam File Tree
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let mut search_buf = state.search_filter.clone();
            let edit_resp = ui.add(
                egui::TextEdit::singleline(&mut search_buf)
                    .hint_text("Cari file & folder...")
                    .desired_width(ui.available_width() - 12.0),
            );

            if edit_resp.changed() {
                *event = Some(SidebarEvent::SearchFilterChanged(search_buf));
            }
        });

        ui.add_space(6.0);
        ui.add(egui::Separator::default().spacing(0.0));
        ui.add_space(4.0);

        // Render Pohon File & Folder
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(tree) = &state.file_tree {
                if tree.children.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("📁 Vault Kosong")
                                .size(12.0)
                                .color(TEXT_MUTED),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Klik '+ Dokumen' atau '+ Folder' di atas untuk mulai membuat catatan.")
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );
                    });
                } else {
                    for child in &tree.children {
                        Self::render_tree_item(ui, child, 0, state, event);
                    }
                }
            } else {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Belum ada Vault dibuka")
                            .size(12.0)
                            .color(TEXT_MUTED),
                    );
                    ui.add_space(6.0);
                    let pick_btn = egui::Button::new(
                        RichText::new(format!("{} Buka Vault", ICON_FOLDER_OPEN.codepoint))
                            .size(11.5)
                            .color(Color32::WHITE),
                    )
                    .fill(ACCENT_BLUE)
                    .corner_radius(CornerRadius::same(ROUNDING_SM));

                    if ui.add(pick_btn).clicked() {
                        *event = Some(SidebarEvent::OpenVaultPicker);
                    }
                });
            }
            ui.add_space(16.0);
        });
    }

    /// Render satu baris item pohon (Folder atau Berkas).
    fn render_tree_item(
        ui: &mut Ui,
        node: &FileTreeNode,
        depth: usize,
        state: &SidebarState,
        event: &mut Option<SidebarEvent>,
    ) {
        // Cek filter pencarian
        if !state.search_filter.is_empty() && !node.matches_filter(&state.search_filter) {
            return;
        }

        let indent = depth as f32 * 14.0 + 4.0;
        let row_height = 26.0;
        let is_dir = node.is_dir;

        if is_dir {
            let is_expanded = state.expanded_folders.contains(&node.path)
                || (!state.search_filter.is_empty() && node.matches_filter(&state.search_filter));

            // Drag and Drop Zone untuk Folder (menerima berkas/folder lain untuk dipindahkan)
            let (rect, resp) = ui.allocate_exact_size(
                Vec2::new(ui.available_width() - 6.0, row_height),
                Sense::click(),
            );

            // Drag-and-drop drop target
            if let Some(payload) = resp.dnd_hover_payload::<PathBuf>() {
                if *payload != node.path && !node.path.starts_with(&*payload) {
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(ROUNDING_SM),
                        Stroke::new(1.5, ACCENT_BLUE),
                        StrokeKind::Inside,
                    );
                }
            }

            if let Some(payload) = resp.dnd_release_payload::<PathBuf>() {
                if *payload != node.path && !node.path.starts_with(&*payload) {
                    *event = Some(SidebarEvent::DirectMove {
                        src_path: (*payload).clone(),
                        dest_dir: node.path.clone(),
                    });
                }
            }

            let is_hovered = resp.hovered();

            if is_hovered {
                ui.painter().rect(
                    rect,
                    CornerRadius::same(ROUNDING_SM),
                    BG_HOVER_DARK,
                    Stroke::new(0.5, BORDER_SUBTLE),
                    StrokeKind::Inside,
                );
            }

            // 1. Chevron icon
            let chevron = if is_expanded { "▼" } else { "▶" };
            let chevron_pos = Pos2::new(rect.min.x + indent, rect.center().y);
            ui.painter().text(
                chevron_pos,
                Align2::LEFT_CENTER,
                chevron,
                egui::FontId::proportional(9.0),
                TEXT_MUTED,
            );

            // 2. Folder icon
            let folder_icon = if is_expanded {
                ICON_FOLDER_OPEN.codepoint
            } else {
                ICON_FOLDER.codepoint
            };
            let folder_icon_pos = Pos2::new(rect.min.x + indent + 12.0, rect.center().y);
            ui.painter().text(
                folder_icon_pos,
                Align2::LEFT_CENTER,
                folder_icon,
                egui::FontId::proportional(14.0),
                ACCENT_BLUE,
            );

            // 3. Folder Name & Count
            let child_count = node.children.len();
            let label_text = format!("{} ({child_count})", node.display_label());
            let text_pos = Pos2::new(rect.min.x + indent + 30.0, rect.center().y);
            ui.painter().text(
                text_pos,
                Align2::LEFT_CENTER,
                &label_text,
                egui::FontId::proportional(12.0),
                TEXT_PRIMARY,
            );

            // 4. Hover Action Buttons pada Folder (+ Catatan, ✏ Rename, ↗ Move, 🗑 Delete)
            if is_hovered {
                let mut action_x = rect.max.x - 6.0;

                // Delete Folder button
                action_x -= 16.0;
                let del_rect = Rect::from_center_size(Pos2::new(action_x, rect.center().y), Vec2::splat(16.0));
                let del_resp = ui.interact(del_rect, ui.id().with(("del_folder", &node.path)), Sense::click());
                ui.painter().text(del_rect.center(), Align2::CENTER_CENTER, ICON_DELETE.codepoint, egui::FontId::proportional(12.0), if del_resp.hovered() { Color32::from_rgb(239, 68, 68) } else { TEXT_MUTED });
                if del_resp.on_hover_text("Hapus Folder").clicked() {
                    *event = Some(SidebarEvent::DeleteItem {
                        path: node.path.clone(),
                        is_dir: true,
                        name: node.display_label().to_string(),
                    });
                }

                // Rename Folder button
                action_x -= 16.0;
                let ren_rect = Rect::from_center_size(Pos2::new(action_x, rect.center().y), Vec2::splat(16.0));
                let ren_resp = ui.interact(ren_rect, ui.id().with(("ren_folder", &node.path)), Sense::click());
                ui.painter().text(ren_rect.center(), Align2::CENTER_CENTER, ICON_EDIT.codepoint, egui::FontId::proportional(12.0), if ren_resp.hovered() { ACCENT_BLUE } else { TEXT_MUTED });
                if ren_resp.on_hover_text("Ganti Nama Folder").clicked() {
                    *event = Some(SidebarEvent::RenameItem {
                        path: node.path.clone(),
                        is_dir: true,
                        current_name: node.name.clone(),
                    });
                }

                // Move Folder button
                action_x -= 16.0;
                let move_rect = Rect::from_center_size(Pos2::new(action_x, rect.center().y), Vec2::splat(16.0));
                let move_resp = ui.interact(move_rect, ui.id().with(("move_folder", &node.path)), Sense::click());
                ui.painter().text(move_rect.center(), Align2::CENTER_CENTER, "↗", egui::FontId::proportional(12.0), if move_resp.hovered() { ACCENT_BLUE } else { TEXT_MUTED });
                if move_resp.on_hover_text("Pindahkan Folder...").clicked() {
                    *event = Some(SidebarEvent::MoveItemPrompt {
                        path: node.path.clone(),
                        is_dir: true,
                        name: node.display_label().to_string(),
                    });
                }

                // Add Note in Folder button
                action_x -= 16.0;
                let add_rect = Rect::from_center_size(Pos2::new(action_x, rect.center().y), Vec2::splat(16.0));
                let add_resp = ui.interact(add_rect, ui.id().with(("add_note_in", &node.path)), Sense::click());
                ui.painter().text(add_rect.center(), Align2::CENTER_CENTER, ICON_NOTE_ADD.codepoint, egui::FontId::proportional(12.0), if add_resp.hovered() { ACCENT_BLUE } else { TEXT_MUTED });
                if add_resp.on_hover_text("Tambah Catatan di Folder Ini").clicked() {
                    *event = Some(SidebarEvent::CreateNote {
                        parent_dir: Some(node.path.clone()),
                    });
                }
            }

            if resp.clicked() {
                *event = Some(SidebarEvent::ToggleFolder(node.path.clone()));
            }

            ui.add_space(1.0);

            // Render anak-anak folder jika terbuka
            if is_expanded {
                for child in &node.children {
                    Self::render_tree_item(ui, child, depth + 1, state, event);
                }
            }
        } else {
            // Render Baris File (Note / Canvas / PDF)
            let is_active = state
                .active_file_path
                .as_ref()
                .map(|p| p == &node.path)
                .unwrap_or(false);

            let (rect, resp) = ui.allocate_exact_size(
                Vec2::new(ui.available_width() - 6.0, row_height),
                Sense::click_and_drag(),
            );

            let is_hovered = resp.hovered();

            // Drag source
            if resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            resp.dnd_set_drag_payload(node.path.clone());

            if is_active {
                ui.painter().rect(
                    rect,
                    CornerRadius::same(ROUNDING_SM),
                    Color32::from_rgba_premultiplied(10, 132, 255, 45),
                    Stroke::new(1.0, ACCENT_BLUE),
                    StrokeKind::Inside,
                );
            } else if is_hovered {
                ui.painter().rect(
                    rect,
                    CornerRadius::same(ROUNDING_SM),
                    BG_HOVER_DARK,
                    Stroke::new(0.5, BORDER_SUBTLE),
                    StrokeKind::Inside,
                );
            }

            // Icon berdasarkan jenis berkas
            let (file_icon, icon_color) = if node.is_canvas {
                (ICON_DRAW.codepoint, Color32::from_rgb(255, 149, 0))
            } else if node.is_pdf {
                (ICON_PICTURE_AS_PDF.codepoint, Color32::from_rgb(255, 69, 58))
            } else {
                (ICON_DESCRIPTION.codepoint, ACCENT_BLUE)
            };

            let icon_pos = Pos2::new(rect.min.x + indent + 12.0, rect.center().y);
            ui.painter().text(
                icon_pos,
                Align2::LEFT_CENTER,
                file_icon,
                egui::FontId::proportional(13.0),
                icon_color,
            );

            // Nama Berkas / Judul Catatan
            let display_name = node.display_label();
            let truncated = if display_name.len() > 24 {
                format!("{}…", &display_name[..21])
            } else {
                display_name.to_string()
            };

            let text_pos = Pos2::new(rect.min.x + indent + 30.0, rect.center().y);
            ui.painter().text(
                text_pos,
                Align2::LEFT_CENTER,
                &truncated,
                egui::FontId::proportional(12.0),
                if is_active { Color32::WHITE } else { TEXT_PRIMARY },
            );

            // Hover action buttons pada File (✏ Rename, ↗ Move, 🗑 Delete)
            if is_hovered {
                let mut action_x = rect.max.x - 6.0;

                // Delete button
                action_x -= 16.0;
                let del_rect = Rect::from_center_size(Pos2::new(action_x, rect.center().y), Vec2::splat(16.0));
                let del_resp = ui.interact(del_rect, ui.id().with(("del_file", &node.path)), Sense::click());
                ui.painter().text(del_rect.center(), Align2::CENTER_CENTER, ICON_DELETE.codepoint, egui::FontId::proportional(12.0), if del_resp.hovered() { Color32::from_rgb(239, 68, 68) } else { TEXT_MUTED });
                if del_resp.on_hover_text("Hapus / Pindahkan ke Sampah").clicked() {
                    *event = Some(SidebarEvent::DeleteItem {
                        path: node.path.clone(),
                        is_dir: false,
                        name: node.display_label().to_string(),
                    });
                }

                // Move button
                action_x -= 16.0;
                let move_rect = Rect::from_center_size(Pos2::new(action_x, rect.center().y), Vec2::splat(16.0));
                let move_resp = ui.interact(move_rect, ui.id().with(("move_file", &node.path)), Sense::click());
                ui.painter().text(move_rect.center(), Align2::CENTER_CENTER, "↗", egui::FontId::proportional(12.0), if move_resp.hovered() { ACCENT_BLUE } else { TEXT_MUTED });
                if move_resp.on_hover_text("Pindahkan ke Folder...").clicked() {
                    *event = Some(SidebarEvent::MoveItemPrompt {
                        path: node.path.clone(),
                        is_dir: false,
                        name: node.display_label().to_string(),
                    });
                }

                // Rename button
                action_x -= 16.0;
                let ren_rect = Rect::from_center_size(Pos2::new(action_x, rect.center().y), Vec2::splat(16.0));
                let ren_resp = ui.interact(ren_rect, ui.id().with(("ren_file", &node.path)), Sense::click());
                ui.painter().text(ren_rect.center(), Align2::CENTER_CENTER, ICON_EDIT.codepoint, egui::FontId::proportional(12.0), if ren_resp.hovered() { ACCENT_BLUE } else { TEXT_MUTED });
                if ren_resp.on_hover_text("Ganti Nama").clicked() {
                    *event = Some(SidebarEvent::RenameItem {
                        path: node.path.clone(),
                        is_dir: false,
                        current_name: node.display_label().to_string(),
                    });
                }
            }

            if resp.clicked() {
                if node.is_pdf {
                    *event = Some(SidebarEvent::OpenPdf(node.path.clone()));
                } else {
                    *event = Some(SidebarEvent::OpenFile(node.path.clone()));
                }
            }

            ui.add_space(1.0);
        }
    }

    /// Render Tab Filter & Kategori Dokumen (Sistem filter, Tag, dan PDF Ingestion).
    fn render_filters_tab(
        ui: &mut Ui,
        state: &SidebarState,
        event: &mut Option<SidebarEvent>,
    ) {
        let vault_btn = egui::Button::new(
            RichText::new(format!("{}  Pilih / Buka Vault...", ICON_FOLDER_OPEN.codepoint))
                .size(11.5)
                .color(TEXT_SECONDARY),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(0.5, BORDER_SUBTLE))
        .corner_radius(CornerRadius::same(ROUNDING_SM));

        ui.horizontal(|ui| {
            ui.add_space(4.0);
            if ui.add_sized(Vec2::new(ui.available_width() - 8.0, 26.0), vault_btn).clicked() {
                *event = Some(SidebarEvent::OpenVaultPicker);
            }
        });

        ui.add_space(6.0);
        ui.add(egui::Separator::default().spacing(0.0));
        ui.add_space(4.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(2.0);

            // ── Kategori Dokumen ──
            let items = [
                (
                    SidebarDocFilter::All,
                    ICON_CATEGORY.codepoint,
                    "Semua Dokumen",
                    state.total_notes + state.total_pdfs + state.total_whiteboards,
                ),
                (
                    SidebarDocFilter::NotesOnly,
                    ICON_DESCRIPTION.codepoint,
                    "Catatan Markdown",
                    state.total_notes,
                ),
                (
                    SidebarDocFilter::WhiteboardsOnly,
                    ICON_DRAW.codepoint,
                    "Whiteboard Kanvas",
                    state.total_whiteboards,
                ),
                (
                    SidebarDocFilter::PdfsOnly,
                    ICON_PICTURE_AS_PDF.codepoint,
                    "Dokumen PDF",
                    state.total_pdfs,
                ),
                (
                    SidebarDocFilter::Archived,
                    ICON_INVENTORY_2.codepoint,
                    "Arsip",
                    state.total_archived,
                ),
                (
                    SidebarDocFilter::Trashed,
                    ICON_DELETE.codepoint,
                    "Tong Sampah",
                    state.total_trashed,
                ),
            ];

            for (filter_kind, icon, label, count) in items {
                let is_active = state.current_filter == filter_kind;
                let text_color = if is_active {
                    Color32::WHITE
                } else {
                    TEXT_SECONDARY
                };

                let bg_fill = if is_active {
                    Color32::from_rgba_premultiplied(10, 132, 255, 60)
                } else {
                    Color32::TRANSPARENT
                };

                let (rect, resp) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width() - 8.0, 28.0),
                    Sense::click(),
                );

                if resp.hovered() && !is_active {
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(ROUNDING_SM),
                        BG_HOVER_DARK,
                        Stroke::new(0.5, BORDER_SUBTLE),
                        StrokeKind::Inside,
                    );
                } else if is_active {
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(ROUNDING_SM),
                        bg_fill,
                        Stroke::new(1.0, ACCENT_BLUE),
                        StrokeKind::Inside,
                    );
                }

                // Icon + Label
                let icon_pos = Pos2::new(rect.min.x + 8.0, rect.center().y);
                ui.painter().text(
                    icon_pos,
                    Align2::LEFT_CENTER,
                    icon,
                    egui::FontId::proportional(14.0),
                    if is_active { ACCENT_BLUE } else { text_color },
                );

                let text_pos = Pos2::new(rect.min.x + 28.0, rect.center().y);
                ui.painter().text(
                    text_pos,
                    Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(12.5),
                    text_color,
                );

                // Count pill
                let count_str = count.to_string();
                let count_pos = Pos2::new(rect.max.x - 8.0, rect.center().y);
                ui.painter().text(
                    count_pos,
                    Align2::RIGHT_CENTER,
                    &count_str,
                    egui::FontId::proportional(11.0),
                    TEXT_MUTED,
                );

                if resp.clicked() {
                    *event = Some(SidebarEvent::SelectFilter(filter_kind));
                }

                ui.add_space(2.0);
            }

            ui.add_space(10.0);
            ui.add(egui::Separator::default().spacing(0.0));
            ui.add_space(6.0);

            // ── Bagian Tag / Label ──
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("LABEL & TAG")
                        .size(10.5)
                        .color(TEXT_MUTED)
                        .strong(),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    let manage_btn = egui::Button::new(
                        RichText::new(format!("{} Kelola", ICON_EDIT.codepoint))
                            .size(10.5)
                            .color(ACCENT_BLUE),
                    )
                    .frame(false);

                    if ui.add(manage_btn).clicked() {
                        *event = Some(SidebarEvent::ManageLabels);
                    }
                });
            });

            ui.add_space(4.0);

            if state.all_tags.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("Belum ada tag")
                            .size(11.5)
                            .color(TEXT_MUTED),
                    );
                });
            } else {
                for (tag, count) in &state.all_tags {
                    let is_active = matches!(&state.current_filter, SidebarDocFilter::Tag(t) if t.eq_ignore_ascii_case(tag));
                    let color = tag_color(tag);

                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width() - 8.0, 26.0),
                        Sense::click(),
                    );

                    if resp.hovered() && !is_active {
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(ROUNDING_SM),
                            BG_HOVER_DARK,
                            Stroke::NONE,
                            StrokeKind::Inside,
                        );
                    } else if is_active {
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(ROUNDING_SM),
                            Color32::from_rgba_premultiplied(10, 132, 255, 40),
                            Stroke::new(1.0, ACCENT_BLUE),
                            StrokeKind::Inside,
                        );
                    }

                    // Dot warna tag
                    let dot_pos = Pos2::new(rect.min.x + 12.0, rect.center().y);
                    ui.painter().circle_filled(dot_pos, 4.0, color);

                    // Tag name
                    let tag_pos = Pos2::new(rect.min.x + 24.0, rect.center().y);
                    ui.painter().text(
                        tag_pos,
                        Align2::LEFT_CENTER,
                        format!("#{tag}"),
                        egui::FontId::proportional(12.0),
                        if is_active { Color32::WHITE } else { TEXT_SECONDARY },
                    );

                    // Count badge
                    let count_pos = Pos2::new(rect.max.x - 8.0, rect.center().y);
                    ui.painter().text(
                        count_pos,
                        Align2::RIGHT_CENTER,
                        count.to_string(),
                        egui::FontId::proportional(11.0),
                        TEXT_MUTED,
                    );

                    if resp.clicked() {
                        *event = Some(SidebarEvent::SelectFilter(SidebarDocFilter::Tag(
                            tag.clone(),
                        )));
                    }

                    ui.add_space(1.0);
                }
            }

            ui.add_space(10.0);
            ui.add(egui::Separator::default().spacing(0.0));
            ui.add_space(6.0);

            // ── Bagian PDF Ingestion ──
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("BERKAS PDF")
                        .size(10.5)
                        .color(TEXT_MUTED)
                        .strong(),
                );
            });

            ui.add_space(4.0);

            let import_btn = egui::Button::new(
                RichText::new(format!("{}  Impor PDF ke Vault", ICON_UPLOAD.codepoint))
                    .size(12.0)
                    .color(Color32::WHITE),
            )
            .fill(ACCENT_BLUE)
            .corner_radius(CornerRadius::same(ROUNDING_SM));

            if ui
                .add_sized(Vec2::new(ui.available_width() - 8.0, 28.0), import_btn)
                .clicked()
            {
                *event = Some(SidebarEvent::ImportPdf);
            }

            ui.add_space(4.0);

            for path in &state.pdf_documents {
                let file_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Dokumen.pdf".to_string());

                let truncated = if file_name.len() > 22 {
                    format!("{}…", &file_name[..19])
                } else {
                    file_name.clone()
                };

                let pdf_btn = egui::Button::new(
                    RichText::new(format!(
                        "{}  {truncated}",
                        ICON_PICTURE_AS_PDF.codepoint
                    ))
                    .size(11.5)
                    .color(TEXT_SECONDARY),
                )
                .frame(false);

                if ui.add(pdf_btn).clicked() {
                    *event = Some(SidebarEvent::OpenPdf(path.clone()));
                }
            }

            ui.add_space(12.0);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_sidebar_state() {
        let state = SidebarState {
            is_open: true,
            active_tab: SidebarTab::Files,
            vault_root: None,
            vault_name: "Test Vault".to_string(),
            active_file_path: None,
            expanded_folders: HashSet::new(),
            file_tree: None,
            search_filter: String::new(),
            current_filter: SidebarDocFilter::All,
            all_tags: vec![("rust".to_string(), 4), ("notes".to_string(), 12)],
            pdf_documents: vec![],
            total_notes: 16,
            total_pdfs: 0,
            total_whiteboards: 1,
            total_archived: 2,
            total_trashed: 0,
        };

        assert!(state.is_open);
        assert_eq!(state.active_tab, SidebarTab::Files);
        assert_eq!(state.current_filter, SidebarDocFilter::All);
        assert_eq!(state.all_tags.len(), 2);
    }

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
        assert!(note.path.exists());
        assert_eq!(note.path.parent().unwrap(), sub1);

        // Move note from Folder A to Folder B
        let target_path = sub2.join(note.path.file_name().unwrap());
        std::fs::rename(&note.path, &target_path).unwrap();

        assert!(!note.path.exists());
        assert!(target_path.exists());

        let reloaded_note = Note::load(&target_path).unwrap();
        assert_eq!(reloaded_note.frontmatter.title, "Task List");

        let notes = vec![reloaded_note];
        let tree = FileTreeNode::build(dir.path(), &notes, &[]).unwrap();

        let folder_b_node = tree.children.iter().find(|c| c.name == "Folder B").unwrap();
        assert_eq!(folder_b_node.children.len(), 1);
        assert_eq!(folder_b_node.children[0].display_label(), "Task List");
    }
}


