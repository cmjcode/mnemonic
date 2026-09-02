//! Komponen UI MNEMONIC terpadu bergaya Shapr3D / DUCAD (Floating Canvas-First UI):
//! - Tema glassmorphism gelap & terang serta token warna (`theme`)
//! - Bilah atas mengambang & navigasi segmen (`top_bar`)
//! - Slide-over drawer samping untuk filter dokumen & tag (`sidebar`)
//! - Bilah alat vertikal mengambang di sisi kiri (`left_toolbar`)
//! - Command palette bergaya VS Code / Spotlight (`command_palette`)
//! - In-Canvas HUD pills untuk zoom & style picker (`canvas_hud`)
//! - Dialog modal & konfirmasi destruktif (`modal`)

pub mod canvas_hud;
pub mod command_palette;
pub mod left_toolbar;
pub mod modal;
pub mod sidebar;
pub mod theme;
pub mod top_bar;

pub use canvas_hud::{CanvasHud, CanvasHudEvent};
pub use command_palette::{CommandPalette, PaletteCommand};
pub use left_toolbar::{LeftToolbar, LeftToolbarEvent};
pub use modal::{ConfirmModal, LabelManagerEvent, LabelManagerModal};
pub use sidebar::{SidebarDocFilter, SidebarDrawer, SidebarEvent, SidebarState};
pub use theme::{
    apply_theme, card_frame, color_for, color_solid_for, glass_frame, glass_panel_frame,
    glass_topbar_frame, pill_frame, tag_chip_frame, tag_color, ThemeMode, ACCENT_BLUE,
    ACCENT_GREEN, ACCENT_ORANGE, ACCENT_PURPLE, BG_CANVAS, BG_CARD_DARK, BG_HOVER_DARK,
    BG_PANEL_DARK, BORDER_SUBTLE, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
pub use top_bar::{TopBar, TopBarEvent, TopBarNavTab, TopBarState};
