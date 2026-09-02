//! Modern Floating Top Bar bergaya Shapr3D / DUCAD dengan Material Icons.
//!
//! Menampilkan bar atas mengambang minimalis dengan nama vault/dokumen,
//! indikator status sinkronisasi, navigasi mode tersegmen, quick note capture,
//! pemicu Command Palette (⌘K), tombol ganti tema, dan pemilih bahasa.

use egui::{
    Align2, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use egui_icons::icons::{
    ICON_AUTO_AWESOME, ICON_CLOSE, ICON_DARK_MODE, ICON_LANGUAGE, ICON_LIGHT_MODE, ICON_MENU,
    ICON_SEARCH,
};

use crate::ui::theme::{
    glass_topbar_frame, ThemeMode, ACCENT_BLUE, BG_CARD_DARK, BG_HOVER_DARK, BORDER_SUBTLE,
    ROUNDING_SM, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopBarNavTab {
    Notes,
    Canvas,
    Chat,
}

#[derive(Debug, Clone)]
pub enum TopBarEvent {
    SelectTab(TopBarNavTab),
    ToggleSidebar,
    ToggleChatSidebar,
    OpenCommandPalette,
    ToggleTheme,
    SetLanguage(String),
    SearchChanged(String),
}

pub struct TopBarState {
    pub vault_name: String,
    pub vault_open: bool,
    pub active_tab: TopBarNavTab,
    pub sidebar_open: bool,
    pub chat_open: bool,
    pub theme_mode: ThemeMode,
    pub active_locale: String,
    pub search_text: String,
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
                // 1. Menu hamburger button -> Toggle unified Sidebar Drawer
                let is_burger_active = state.sidebar_open;
                let burger_color = if is_burger_active {
                    ACCENT_BLUE
                } else {
                    TEXT_PRIMARY
                };
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                let is_hovered = resp.hovered();

                if is_hovered || is_burger_active {
                    let fill = if is_burger_active {
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
                            if is_burger_active {
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
                    ICON_MENU.codepoint,
                    egui::FontId::proportional(icon_sz),
                    burger_color,
                );

                if resp
                    .on_hover_text("Menu Dokumen & Vault")
                    .clicked()
                {
                    event = Some(TopBarEvent::ToggleSidebar);
                }

                // 2. Search Input Field (Langsung di Header Menu - Tanpa border dan icon)
                if state.vault_open {
                    ui.add_space(8.0);

                    let edit = egui::TextEdit::singleline(&mut state.search_text)
                        .hint_text("Cari catatan, tag, berkas...")
                        .frame(Frame::NONE)
                        .desired_width(180.0);
                    let resp = ui.add(edit);

                    if resp.changed() {
                        event = Some(TopBarEvent::SearchChanged(state.search_text.clone()));
                    }

                    if !state.search_text.is_empty() {
                        let clear_btn = egui::Button::new(
                            RichText::new(ICON_CLOSE.codepoint)
                                .size(10.5)
                                .color(TEXT_MUTED),
                        )
                        .frame(false);

                        if ui.add(clear_btn).on_hover_text("Hapus pencarian").clicked() {
                            state.search_text.clear();
                            event = Some(TopBarEvent::SearchChanged(String::new()));
                        }
                    }
                }

                // 4. Right Layout: AI Chat Toggle Icon + Theme Switcher + Language Selector + Omnibox ⌘K
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 4a. AI Chat Toggle Icon (Ujung Kanan Header)
                    if state.vault_open {
                        let is_chat_active = state.chat_open;
                        let chat_color = if is_chat_active {
                            ACCENT_BLUE
                        } else {
                            TEXT_SECONDARY
                        };
                        let (c_rect, c_resp) =
                            ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                        let is_c_hovered = c_resp.hovered();

                        if is_c_hovered || is_chat_active {
                            let fill = if is_chat_active {
                                Color32::from_rgba_premultiplied(10, 132, 255, 45)
                            } else {
                                BG_HOVER_DARK
                            };
                            ui.painter().rect(
                                c_rect,
                                CornerRadius::same(ROUNDING_SM),
                                fill,
                                Stroke::new(
                                    0.5,
                                    if is_chat_active {
                                        ACCENT_BLUE
                                    } else {
                                        BORDER_SUBTLE
                                    },
                                ),
                                StrokeKind::Inside,
                            );
                        }

                        ui.painter().text(
                            c_rect.center(),
                            Align2::CENTER_CENTER,
                            ICON_AUTO_AWESOME.codepoint,
                            egui::FontId::proportional(icon_sz),
                            chat_color,
                        );

                        if c_resp
                            .on_hover_text("Buka / Tutup Asisten AI")
                            .clicked()
                        {
                            event = Some(TopBarEvent::ToggleChatSidebar);
                        }

                        ui.add_space(2.0);
                    }

                    // 4b. Theme Switcher button
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

                    // 4c. Language Selector Dropdown
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

                    // 4d. Omnibox Command Palette Trigger (⌘K)
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
                                    RichText::new("⌘K")
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
            chat_open: false,
            theme_mode: ThemeMode::Dark,
            active_locale: "id-ID".to_string(),
            search_text: String::new(),
            icon_size: 16.0,
            note_count: 5,
            pdf_count: 2,
        };

        assert_eq!(state.vault_name, "MyVault");
        assert_eq!(state.active_tab, TopBarNavTab::Notes);
        assert!(!state.chat_open);
        assert_eq!(state.theme_mode, ThemeMode::Dark);
        assert_eq!(state.search_text, "");
    }
}
