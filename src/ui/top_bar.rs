//! Modern Floating Top Bar bergaya Shapr3D / DUCAD dengan Material Icons.
//!
//! Menampilkan bar atas mengambang minimalis dengan nama vault/dokumen,
//! indikator status sinkronisasi, navigasi mode tersegmen, quick note capture,
//! pemicu Command Palette (⌘K), tombol ganti tema, dan pemilih bahasa.

use egui::{
    Align2, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use egui_icons::icons::{
    ICON_ADD, ICON_AUTO_AWESOME, ICON_DARK_MODE, ICON_DESCRIPTION, ICON_DRAW, ICON_FOLDER_OPEN,
    ICON_LANGUAGE, ICON_LIGHT_MODE, ICON_MENU, ICON_NOTE_ADD, ICON_PALETTE, ICON_SEARCH,
    ICON_UPLOAD,
};

use crate::ui::theme::{
    glass_frame, glass_topbar_frame, ThemeMode, ACCENT_BLUE, BG_CARD_DARK, BG_HOVER_DARK,
    BORDER_SUBTLE, ROUNDING_SM, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopBarNavTab {
    Notes,
    Canvas,
    Search,
    Chat,
}

#[derive(Debug, Clone)]
pub enum TopBarEvent {
    SelectTab(TopBarNavTab),
    ToggleSidebar,
    OpenVaultPicker,
    CreateNote(String),
    OpenCommandPalette,
    ToggleTheme,
    SetLanguage(String),
    ImportPdf,
    ManageLabels,
    OpenSettings,
    NewCanvas,
}

pub struct TopBarState {
    pub vault_name: String,
    pub vault_open: bool,
    pub active_tab: TopBarNavTab,
    pub sidebar_open: bool,
    pub theme_mode: ThemeMode,
    pub active_locale: String,
    pub quick_capture_text: String,
    pub icon_size: f32,
    pub note_count: usize,
    pub pdf_count: usize,
}

pub struct TopBar;

impl TopBar {
    /// Render TopBar floating pill-bar. Mengembalikan `Option<TopBarEvent>`.
    pub fn show(ui: &mut Ui, state: &mut TopBarState) -> Option<TopBarEvent> {
        let mut event = None;
        let icon_sz = state.icon_size.clamp(14.0, 18.0);

        glass_topbar_frame().show(ui, |ui| {
            ui.set_height(34.0);
            ui.horizontal(|ui| {
                // 1. Menu hamburger / Vault operations dropdown
                ui.menu_button(
                    RichText::new(ICON_MENU.codepoint)
                        .size(icon_sz)
                        .color(TEXT_PRIMARY),
                    |ui| {
                        ui.set_min_width(180.0);
                        if ui
                            .button(format!(
                                "{}  Pilih / Buka Vault...",
                                ICON_FOLDER_OPEN.codepoint
                            ))
                            .clicked()
                        {
                            event = Some(TopBarEvent::OpenVaultPicker);
                            ui.close();
                        }
                        if state.vault_open {
                            if ui
                                .button(format!("{}  Catatan Baru (⌘N)", ICON_NOTE_ADD.codepoint))
                                .clicked()
                            {
                                event = Some(TopBarEvent::CreateNote(String::new()));
                                ui.close();
                            }
                            if ui
                                .button(format!("{}  Kanvas Baru", ICON_DRAW.codepoint))
                                .clicked()
                            {
                                event = Some(TopBarEvent::NewCanvas);
                                ui.close();
                            }
                            if ui
                                .button(format!("{}  Impor PDF...", ICON_UPLOAD.codepoint))
                                .clicked()
                            {
                                event = Some(TopBarEvent::ImportPdf);
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .button(format!("{}  Kelola Label...", ICON_PALETTE.codepoint))
                                .clicked()
                            {
                                event = Some(TopBarEvent::ManageLabels);
                                ui.close();
                            }
                        }
                    },
                );

                // 2. Sidebar drawer toggle button (hanya bila vault aktif)
                if state.vault_open {
                    let sidebar_color = if state.sidebar_open {
                        ACCENT_BLUE
                    } else {
                        TEXT_SECONDARY
                    };
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                    let is_hovered = resp.hovered();

                    if is_hovered || state.sidebar_open {
                        let fill = if state.sidebar_open {
                            Color32::from_rgba_premultiplied(10, 132, 255, 45)
                        } else {
                            BG_HOVER_DARK
                        };
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(ROUNDING_SM),
                            fill,
                            Stroke::new(
                                0.5,
                                if state.sidebar_open {
                                    ACCENT_BLUE
                                } else {
                                    BORDER_SUBTLE
                                },
                            ),
                            StrokeKind::Inside,
                        );
                    }

                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        ICON_DESCRIPTION.codepoint,
                        egui::FontId::proportional(icon_sz),
                        sidebar_color,
                    );

                    if resp
                        .on_hover_text("Buka/Tutup Bilah Samping (Sidebar)")
                        .clicked()
                    {
                        event = Some(TopBarEvent::ToggleSidebar);
                    }
                }

                ui.add_space(4.0);

                // 3. App Title & Vault Name badge
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("MNEMONIC")
                            .size(13.5)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );

                    if state.vault_open {
                        let vault_label = if state.vault_name.is_empty() {
                            "Vault".to_string()
                        } else {
                            state.vault_name.clone()
                        };
                        let pill_text = RichText::new(format!("📁 {vault_label}"))
                            .size(11.0)
                            .color(TEXT_SECONDARY);

                        let pill_frame = Frame {
                            inner_margin: Margin::symmetric(6, 2),
                            outer_margin: Margin::ZERO,
                            corner_radius: CornerRadius::same(ROUNDING_SM),
                            fill: BG_CARD_DARK,
                            stroke: Stroke::new(0.5, BORDER_SUBTLE),
                            shadow: egui::Shadow::NONE,
                        };

                        pill_frame.show(ui, |ui| {
                            ui.label(pill_text);
                        });
                    }
                });

                // 4. Center Segmented Navigation Pills (Notes, Canvas, Search, Chat)
                if state.vault_open {
                    ui.add_space(8.0);
                    let tabs = [
                        (TopBarNavTab::Notes, ICON_DESCRIPTION.codepoint, "Catatan"),
                        (TopBarNavTab::Canvas, ICON_DRAW.codepoint, "Kanvas"),
                        (TopBarNavTab::Search, ICON_SEARCH.codepoint, "Pencarian"),
                        (TopBarNavTab::Chat, ICON_AUTO_AWESOME.codepoint, "AI Chat"),
                    ];

                    for (tab, icon, label) in tabs {
                        let is_active = state.active_tab == tab;
                        let text_color = if is_active {
                            Color32::WHITE
                        } else {
                            TEXT_SECONDARY
                        };

                        let bg_color = if is_active {
                            ACCENT_BLUE
                        } else {
                            Color32::TRANSPARENT
                        };

                        let btn_frame = Frame {
                            inner_margin: Margin::symmetric(8, 3),
                            outer_margin: Margin::ZERO,
                            corner_radius: CornerRadius::same(ROUNDING_SM),
                            fill: bg_color,
                            stroke: if is_active {
                                Stroke::new(1.0, ACCENT_BLUE)
                            } else {
                                Stroke::NONE
                            },
                            shadow: egui::Shadow::NONE,
                        };

                        let resp = btn_frame
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(icon)
                                            .size(icon_sz - 1.0)
                                            .color(text_color),
                                    );
                                    ui.label(
                                        RichText::new(label).size(12.5).color(text_color),
                                    );
                                });
                            })
                            .response;

                        if resp.interact(Sense::click()).clicked() {
                            event = Some(TopBarEvent::SelectTab(tab));
                        }
                    }
                }

                // 5. Right Layout: Quick Capture + Omnibox ⌘K + Theme + Language
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Theme Switcher button
                    let theme_icon = match state.theme_mode {
                        ThemeMode::Dark => ICON_LIGHT_MODE.codepoint,
                        ThemeMode::Light => ICON_DARK_MODE.codepoint,
                    };
                    let (t_rect, t_resp) =
                        ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                    if t_resp.hovered() {
                        ui.painter().rect(
                            t_rect,
                            CornerRadius::same(ROUNDING_SM),
                            BG_HOVER_DARK,
                            Stroke::new(0.5, BORDER_SUBTLE),
                            StrokeKind::Inside,
                        );
                    }
                    ui.painter().text(
                        t_rect.center(),
                        Align2::CENTER_CENTER,
                        theme_icon,
                        egui::FontId::proportional(icon_sz),
                        TEXT_SECONDARY,
                    );
                    if t_resp.on_hover_text("Ganti Tema (Gelap / Terang)").clicked() {
                        event = Some(TopBarEvent::ToggleTheme);
                    }

                    // Language Selector Dropdown
                    ui.menu_button(
                        RichText::new(ICON_LANGUAGE.codepoint)
                            .size(icon_sz)
                            .color(TEXT_SECONDARY),
                        |ui| {
                            if ui.button("🇮🇩 Bahasa Indonesia (id-ID)").clicked() {
                                event = Some(TopBarEvent::SetLanguage("id-ID".to_string()));
                                ui.close();
                            }
                            if ui.button("🇺🇸 English (en-US)").clicked() {
                                event = Some(TopBarEvent::SetLanguage("en-US".to_string()));
                                ui.close();
                            }
                        },
                    );

                    ui.add_space(4.0);

                    // Omnibox Command Palette Trigger (⌘K)
                    let palette_btn_frame = Frame {
                        inner_margin: Margin::symmetric(8, 3),
                        outer_margin: Margin::ZERO,
                        corner_radius: CornerRadius::same(ROUNDING_SM),
                        fill: BG_CARD_DARK,
                        stroke: Stroke::new(0.5, BORDER_SUBTLE),
                        shadow: egui::Shadow::NONE,
                    };

                    let pal_resp = palette_btn_frame
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(ICON_SEARCH.codepoint)
                                        .size(13.0)
                                        .color(TEXT_MUTED),
                                );
                                ui.label(
                                    RichText::new("Cari atau ⌘K")
                                        .size(12.0)
                                        .color(TEXT_MUTED),
                                );
                            });
                        })
                        .response;

                    if pal_resp
                        .interact(Sense::click())
                        .on_hover_text("Buka Command Palette (Cmd+K / Ctrl+K)")
                        .clicked()
                    {
                        event = Some(TopBarEvent::OpenCommandPalette);
                    }

                    // Quick Note Capture input (hanya saat mode Notes)
                    if state.vault_open && state.active_tab == TopBarNavTab::Notes {
                        ui.add_space(4.0);

                        // Tombol submit tambah
                        let add_icon_btn = ui.add(
                            egui::Button::new(
                                RichText::new(ICON_ADD.codepoint)
                                    .size(icon_sz)
                                    .color(Color32::WHITE),
                            )
                            .fill(ACCENT_BLUE)
                            .corner_radius(CornerRadius::same(ROUNDING_SM)),
                        );

                        let should_submit = add_icon_btn.clicked()
                            && !state.quick_capture_text.trim().is_empty();

                        // Input field
                        let edit = egui::TextEdit::singleline(&mut state.quick_capture_text)
                            .hint_text("✏ Judul cepat...")
                            .desired_width(140.0);
                        let edit_resp = ui.add(edit);

                        if (should_submit
                            || (edit_resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                            && !state.quick_capture_text.trim().is_empty()
                        {
                            event = Some(TopBarEvent::CreateNote(
                                state.quick_capture_text.trim().to_string(),
                            ));
                            state.quick_capture_text.clear();
                        }
                    }
                });
            });
        });

        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topbar_state_init() {
        let state = TopBarState {
            vault_name: "MyVault".to_string(),
            vault_open: true,
            active_tab: TopBarNavTab::Notes,
            sidebar_open: false,
            theme_mode: ThemeMode::Dark,
            active_locale: "id-ID".to_string(),
            quick_capture_text: String::new(),
            icon_size: 16.0,
            note_count: 5,
            pdf_count: 2,
        };

        assert_eq!(state.vault_name, "MyVault");
        assert_eq!(state.active_tab, TopBarNavTab::Notes);
        assert_eq!(state.theme_mode, ThemeMode::Dark);
    }
}
