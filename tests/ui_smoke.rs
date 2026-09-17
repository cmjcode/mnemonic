//! Headless UI smoke test: runs the real app against a throwaway vault,
//! walks through the main screens with simulated keyboard input, and makes
//! sure every frame lays out without panicking — in both themes.
//!
//! Set `MNEMONIC_SNAPSHOT_DIR=/some/dir` to also save a PNG of each screen
//! (rendered offscreen with wgpu) for visual review.

use std::path::{Path, PathBuf};

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use mnemonic::app::MnemonicApp;
use mnemonic::notes::Note;

const FRAMES_PER_STEP: usize = 4;

fn config_dirs(home: &Path) -> [PathBuf; 2] {
    // `dirs::config_dir()` is `$HOME/Library/Application Support` on macOS
    // and `$XDG_CONFIG_HOME` (or `$HOME/.config`) on Linux. Cover both.
    [
        home.join("Library/Application Support/mnemonic"),
        home.join(".config/mnemonic"),
    ]
}

fn write_config(home: &Path, contents: &str) {
    for dir in config_dirs(home) {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), contents).unwrap();
    }
}

fn vault_config(vault: &Path, theme: &str, locale: &str) -> String {
    let vault = vault.to_string_lossy();
    format!(
        "vault_path = {vault:?}\nrecent_vaults = [{vault:?}]\ntheme = \"{theme}\"\nlocale = \"{locale}\"\nsidebar_open = true\nshow_outline = true\n"
    )
}

fn seed_vault(vault: &Path) {
    let projects = vault.join("Proyek");
    std::fs::create_dir_all(&projects).unwrap();
    let mut plan = Note::create(
        &projects,
        "Rencana Peluncuran Q4",
        "Target rilis akhir Oktober.\n\n- [x] Riset\n- [x] Desain\n- [ ] Uji coba\n",
    )
    .unwrap();
    plan.frontmatter.pinned = true;
    plan.frontmatter.color = Some("blue".into());
    plan.frontmatter.tags = vec!["kerja".into(), "prioritas".into()];
    plan.save().unwrap();

    let mut recipe = Note::create(
        vault,
        "Ide Resep Minggu Ini",
        "Nasi goreng, sayur asem, tempe mendoan.",
    )
    .unwrap();
    recipe.frontmatter.tags = vec!["rumah".into()];
    recipe.frontmatter.color = Some("yellow".into());
    recipe.save().unwrap();

    Note::create(
        vault,
        "Catatan Kuliah — Struktur Data",
        "# Pohon Biner\nSetiap node punya dua anak.\n\n# Hash Table\nRata-rata O(1).",
    )
    .unwrap();
    Note::create_canvas(vault, "Diagram Arsitektur").unwrap();

    // Something in the trash, so the Trash view has content.
    let old = Note::create(vault, "Draft Lama", "tidak dipakai").unwrap();
    old.move_to_trash(vault).unwrap();
}

fn count_notes(vault: &Path) -> usize {
    walkdir::WalkDir::new(vault)
        .into_iter()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .count()
}

fn snapshot(harness: &mut Harness<'_, MnemonicApp>, name: &str) {
    let Some(dir) = std::env::var_os("MNEMONIC_SNAPSHOT_DIR").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    match harness.render() {
        Ok(image) => image.save(dir.join(format!("{name}.png"))).unwrap(),
        Err(e) => eprintln!("snapshot {name}: render unavailable ({e})"),
    }
}

fn step(harness: &mut Harness<'_, MnemonicApp>) {
    harness.run_steps(FRAMES_PER_STEP);
}

fn cmd(harness: &mut Harness<'_, MnemonicApp>, key: Key) {
    harness.key_press_modifiers(Modifiers::COMMAND, key);
    step(harness);
}

fn press(harness: &mut Harness<'_, MnemonicApp>, key: Key) {
    harness.key_press(key);
    step(harness);
}

fn type_text(harness: &mut Harness<'_, MnemonicApp>, text: &str) {
    harness.event(Event::Text(text.to_string()));
    step(harness);
}

fn build(size: [f32; 2]) -> Harness<'static, MnemonicApp> {
    Harness::builder().with_size(size).build_eframe(|cc| {
        mnemonic::ui::theme::install_fonts(&cc.egui_ctx);
        MnemonicApp::new()
    })
}

fn walk_screens(prefix: &str, vault: &Path) {
    let notes_before = count_notes(vault);
    let mut h = build([1280.0, 800.0]);
    step(&mut h);
    snapshot(&mut h, &format!("{prefix}-01-home"));

    // Search from the top bar.
    cmd(&mut h, Key::F);
    type_text(&mut h, "resep");
    snapshot(&mut h, &format!("{prefix}-02-search"));
    press(&mut h, Key::Escape);

    // Command palette with quick-open.
    cmd(&mut h, Key::K);
    snapshot(&mut h, &format!("{prefix}-03a-palette-empty"));
    type_text(&mut h, "catatan");
    snapshot(&mut h, &format!("{prefix}-03-palette"));
    press(&mut h, Key::Escape);

    // New note: the title is focused & selected, so typing names it.
    cmd(&mut h, Key::N);
    step(&mut h);
    type_text(&mut h, "Belanja Mingguan");
    snapshot(&mut h, &format!("{prefix}-04-editor"));

    // Shortcuts cheat sheet.
    cmd(&mut h, Key::Slash);
    snapshot(&mut h, &format!("{prefix}-05-shortcuts"));
    press(&mut h, Key::Escape);

    // AI assistant panel.
    cmd(&mut h, Key::J);
    snapshot(&mut h, &format!("{prefix}-06-ai-panel"));
    cmd(&mut h, Key::J);

    // Back home (Esc twice: leave the text field, then the document).
    press(&mut h, Key::Escape);
    press(&mut h, Key::Escape);
    snapshot(&mut h, &format!("{prefix}-07-home-after-edit"));

    // Narrow window: layout must still hold together.
    h.set_size(egui::vec2(820.0, 600.0));
    step(&mut h);
    snapshot(&mut h, &format!("{prefix}-08-narrow"));

    assert!(
        count_notes(vault) > notes_before,
        "the new note should have been created on disk"
    );
    // Typing a title and leaving with Esc must keep the title, not revert it.
    let titled = walkdir::WalkDir::new(vault)
        .into_iter()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter_map(|e| Note::load(e.path()).ok())
        .any(|n| n.frontmatter.title == "Belanja Mingguan");
    assert!(titled, "typed title should be saved after pressing Esc");
}

#[test]
fn ui_walkthrough_renders_every_screen_in_both_themes() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    seed_vault(&vault);

    // SAFETY: this test binary contains a single test, so nothing else
    // reads the environment concurrently.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
    }

    write_config(&home, &vault_config(&vault, "dark", "id-ID"));
    walk_screens("dark", &vault);

    write_config(&home, &vault_config(&vault, "light", "en-US"));
    walk_screens("light", &vault);

    // Welcome screen (no vault configured).
    write_config(&home, "theme = \"dark\"\n");
    let mut h = build([1100.0, 760.0]);
    step(&mut h);
    snapshot(&mut h, "welcome");
}
