//! MNEMONIC UI components, all built on one design system:
//! - `theme`: palettes (dark & light), type scale, spacing, frames
//! - `widgets`: shared buttons, rows, inputs, empty states
//! - `top_bar`: docked, context-aware app bar (search / editor / PDF)
//! - `sidebar`: vault switcher, library filters, folder tree, labels
//! - `chat_sidebar`: docked AI assistant panel
//! - `command_palette`: ⌘K command & note switcher
//! - `left_toolbar`, `canvas_hud`: whiteboard tool dock & HUD pills
//! - `modal`: confirmation / prompt / picker dialogs
//! - `toast`: non-blocking notifications with undo

pub mod canvas_hud;
pub mod chat_sidebar;
pub mod command_palette;
pub mod left_toolbar;
pub mod logo;
pub mod modal;
pub mod sidebar;
pub mod theme;
pub mod toast;
pub mod top_bar;
pub mod widgets;

pub use canvas_hud::{CanvasHud, CanvasHudEvent};
pub use chat_sidebar::{
    ChatCitationItem, ChatMessageItem, ChatRole, ChatSidebarDrawer, ChatSidebarEvent,
    ChatSidebarState,
};
pub use command_palette::{CommandPalette, PaletteCommand};
pub use left_toolbar::{LeftToolbar, LeftToolbarEvent};
pub use logo::{load_app_icon_arc, load_app_icon_data, load_logo_color_image, logo_texture};
pub use modal::{
    ConfirmModal, LabelManagerEvent, LabelManagerModal, MoveChoice, MoveFolderModal,
    PromptInputModal, ShortcutsModal,
};
pub use sidebar::{
    FileTreeNode, SidebarCounts, SidebarDocFilter, SidebarDrawer, SidebarEvent, SidebarState,
};
pub use theme::{ThemeMode, apply_theme, pal};
pub use toast::{ToastKind, Toasts};
pub use top_bar::{EditorModeTab, SaveState, TopBar, TopBarContext, TopBarEvent, TopBarState};
