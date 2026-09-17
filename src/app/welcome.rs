//! First-run / no-vault screen: create a new vault (seeded with a welcome
//! note) or open an existing folder, plus one-click recent vaults.

use std::path::PathBuf;

use egui::{Align, Layout, RichText, Vec2};
use egui_icons::icons::{
    ICON_AUTO_AWESOME, ICON_CREATE_NEW_FOLDER, ICON_DRAW, ICON_FOLDER, ICON_FOLDER_OPEN, ICON_LOCK,
};

use super::MnemonicApp;
use crate::ui::{pal, theme, widgets};

enum WelcomeAction {
    Create,
    Open,
    OpenRecent(PathBuf),
}

impl MnemonicApp {
    pub(super) fn show_welcome(&mut self, ui: &mut egui::Ui) {
        let tr = &self.locales;
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let mut action = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = (ui.available_width() - 2.0 * theme::SPACE_XL).min(520.0);
                ui.add_space((ui.available_height() * 0.12).max(theme::SPACE_XL));
                ui.vertical_centered(|ui| {
                    let logo = crate::ui::logo::logo_texture(ui.ctx());
                    ui.add(egui::Image::new((logo.id(), Vec2::splat(72.0))).corner_radius(16.0));
                    ui.add_space(theme::SPACE_L);
                    ui.label(
                        RichText::new(t("welcome-title"))
                            .font(theme::semibold(theme::TEXT_DISPLAY))
                            .color(p.text),
                    );
                    ui.add_space(theme::SPACE_XS);
                    ui.label(
                        RichText::new(t("welcome-subtitle"))
                            .size(theme::TEXT_BODY + 1.0)
                            .color(p.text_dim),
                    );
                    ui.add_space(theme::SPACE_XL);

                    // Primary actions, centered as a group.
                    let create = t("welcome-create-vault");
                    let open = t("welcome-open-folder");
                    let group = ui
                        .scope(|ui| {
                            ui.horizontal(|ui| {
                                let row_w = ui
                                    .ctx()
                                    .data(|d| d.get_temp::<f32>(egui::Id::new("welcome_row_w")))
                                    .unwrap_or(0.0);
                                ui.add_space(((ui.available_width() - row_w) / 2.0).max(0.0));
                                let start = ui.cursor().min.x;
                                if widgets::primary_button(
                                    ui,
                                    Some(ICON_CREATE_NEW_FOLDER.codepoint),
                                    &create,
                                )
                                .clicked()
                                {
                                    action = Some(WelcomeAction::Create);
                                }
                                if widgets::secondary_button(
                                    ui,
                                    Some(ICON_FOLDER_OPEN.codepoint),
                                    &open,
                                )
                                .clicked()
                                {
                                    action = Some(WelcomeAction::Open);
                                }
                                ui.cursor().min.x - start
                            })
                            .inner
                        })
                        .inner;
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(egui::Id::new("welcome_row_w"), group));

                    let recent = &self.derived.recent_vaults;
                    if !recent.is_empty() {
                        ui.add_space(theme::SPACE_XL);
                        ui.scope(|ui| {
                            ui.set_max_width(width);
                            theme::card_frame().show(ui, |ui| {
                                ui.set_width(width - 30.0);
                                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    widgets::section_header(ui, &t("sidebar-recent-vaults"));
                                    for vault in recent {
                                        let name = vault
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_else(|| vault.display().to_string());
                                        let resp = widgets::list_row(
                                            ui,
                                            widgets::RowSpec {
                                                icon: ICON_FOLDER.codepoint,
                                                icon_color: p.folder_icon,
                                                label: &name,
                                                trailing: None,
                                                selected: false,
                                                indent: 0.0,
                                                reserve_right: 0.0,
                                            },
                                        )
                                        .on_hover_text(vault.display().to_string());
                                        if resp.clicked() {
                                            action = Some(WelcomeAction::OpenRecent(vault.clone()));
                                        }
                                    }
                                });
                            });
                        });
                    }

                    ui.add_space(theme::SPACE_XL);
                    for (icon, key) in [
                        (ICON_DRAW.codepoint, "welcome-feature-notes"),
                        (ICON_AUTO_AWESOME.codepoint, "welcome-feature-ai"),
                        (ICON_LOCK.codepoint, "welcome-feature-private"),
                    ] {
                        ui.label(
                            RichText::new(format!("{icon}  {}", t(key)))
                                .size(theme::TEXT_SM)
                                .color(p.text_dim),
                        );
                        ui.add_space(2.0);
                    }
                    ui.add_space(theme::SPACE_XL);
                });
            });

        match action {
            Some(WelcomeAction::Create) => self.create_vault(),
            Some(WelcomeAction::Open) => self.pick_and_open_vault(),
            Some(WelcomeAction::OpenRecent(path)) => self.open_vault_at(path),
            None => {}
        }
    }
}
