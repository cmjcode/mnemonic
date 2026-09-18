//! User-configurable keyboard shortcuts (§Fase 1.8, Obsidian's Hotkeys
//! settings): the `[hotkeys]` table in `config.toml` maps an action id to a
//! chord such as `"Cmd+Shift+K"`; anything not listed falls back to the
//! built-in default. Parsing/formatting is pure so it's unit-testable;
//! `app` consumes the chords each frame. Callers: `app`, `settings`.

use egui::{Key, KeyboardShortcut, Modifiers};

/// `(action id, default chord)` — the ids are what `config.toml` uses.
pub const DEFAULT_HOTKEYS: &[(&str, &str)] = &[
    ("palette", "Cmd+K"),
    ("new_note", "Cmd+N"),
    ("search", "Cmd+F"),
    ("save", "Cmd+S"),
    ("toggle_read", "Cmd+E"),
    ("sidebar", "Cmd+\\"),
    ("ai", "Cmd+J"),
    ("shortcuts", "Cmd+/"),
    ("graph", "Cmd+G"),
    ("daily", "Cmd+D"),
];

/// Locale key describing each action, for the cheat sheet.
pub const ACTION_LABELS: &[(&str, &str)] = &[
    ("palette", "shortcut-palette"),
    ("new_note", "shortcut-new-note"),
    ("search", "shortcut-search"),
    ("save", "shortcut-save"),
    ("toggle_read", "shortcut-toggle-read"),
    ("sidebar", "shortcut-sidebar"),
    ("ai", "shortcut-ai"),
    ("graph", "shortcut-graph"),
    ("daily", "shortcut-daily"),
    ("shortcuts", "shortcut-cheatsheet"),
];

/// The built-in chord for `action`.
pub fn default_chord(action: &str) -> Option<&'static str> {
    DEFAULT_HOTKEYS
        .iter()
        .find(|(a, _)| *a == action)
        .map(|(_, c)| *c)
}

/// Parses `"Cmd+Shift+K"`, `"Ctrl+N"`, `"Alt+F1"`, `"Cmd+\"` … Modifier
/// names: `Cmd`/`Command`/`Meta`/`Super` (⌘ on macOS, Ctrl elsewhere),
/// `Ctrl`/`Control`, `Shift`, `Alt`/`Option`. The key is a letter, digit,
/// punctuation character or an egui key name (`Enter`, `F5`, `Space`).
pub fn parse_chord(chord: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let mut key: Option<Key> = None;
    let parts: Vec<&str> = chord.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }
    // `Cmd++` means the key is `+` itself (split gives two empty tails);
    // a lone trailing `+` (`Cmd+`) is an incomplete chord.
    let n = parts.len();
    let (mods, key_part) = if n >= 3 && parts[n - 1].is_empty() && parts[n - 2].is_empty() {
        (&parts[..n - 2], "+")
    } else {
        (&parts[..n - 1], *parts.last()?)
    };
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" | "super" => modifiers |= Modifiers::COMMAND,
            "ctrl" | "control" => modifiers |= Modifiers::CTRL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" | "option" => modifiers |= Modifiers::ALT,
            _ => return None,
        }
    }
    if key_part.is_empty() {
        return None;
    }
    let name = match key_part {
        "\\" => "Backslash",
        "/" => "Slash",
        "," => "Comma",
        "." => "Period",
        "-" => "Minus",
        "=" => "Equals",
        "+" => "Plus",
        ";" => "Semicolon",
        "'" => "Quote",
        "`" => "Backtick",
        "[" => "OpenBracket",
        "]" => "CloseBracket",
        "esc" | "Esc" => "Escape",
        other => other,
    };
    let upper = name.to_uppercase();
    key = key.or_else(|| Key::from_name(name)).or_else(|| Key::from_name(&upper));
    Some(KeyboardShortcut::new(modifiers, key?))
}

/// Human label for a chord: `"Cmd+Shift+K"` → `"⌘⇧K"`.
pub fn display(chord: &str) -> String {
    let Some(shortcut) = parse_chord(chord) else {
        return chord.to_string();
    };
    let m = shortcut.modifiers;
    let mut out = String::new();
    if m.ctrl {
        out.push('^');
    }
    if m.alt {
        out.push('⌥');
    }
    if m.shift {
        out.push('⇧');
    }
    if m.command || m.mac_cmd {
        out.push('⌘');
    }
    out.push_str(&key_glyph(shortcut.logical_key));
    out
}

fn key_glyph(key: Key) -> String {
    match key {
        Key::Backslash => "\\".into(),
        Key::Slash => "/".into(),
        Key::Comma => ",".into(),
        Key::Period => ".".into(),
        Key::Minus => "-".into(),
        Key::Equals => "=".into(),
        Key::Plus => "+".into(),
        Key::Semicolon => ";".into(),
        Key::Quote => "'".into(),
        Key::Backtick => "`".into(),
        Key::OpenBracket => "[".into(),
        Key::CloseBracket => "]".into(),
        Key::Space => "Space".into(),
        Key::Enter => "↩".into(),
        Key::Escape => "Esc".into(),
        other => other.symbol_or_name().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifier_chords_and_symbols() {
        let s = parse_chord("Cmd+Shift+K").unwrap();
        assert!(s.modifiers.command && s.modifiers.shift);
        assert_eq!(s.logical_key, Key::K);
        assert_eq!(parse_chord("ctrl+n").unwrap().logical_key, Key::N);
        assert_eq!(parse_chord("Cmd+\\").unwrap().logical_key, Key::Backslash);
        assert_eq!(parse_chord("Cmd+/").unwrap().logical_key, Key::Slash);
        assert_eq!(parse_chord("Alt+F5").unwrap().logical_key, Key::F5);
        assert!(parse_chord("Bogus+K").is_none());
        assert!(parse_chord("").is_none());
        assert!(parse_chord("Cmd+").is_none());
        assert_eq!(parse_chord("Cmd++").unwrap().logical_key, Key::Plus);
    }

    #[test]
    fn displays_mac_style_glyphs() {
        assert_eq!(display("Cmd+K"), "⌘K");
        assert_eq!(display("Cmd+Shift+\\"), "⇧⌘\\");
        assert_eq!(display("Ctrl+Alt+Enter"), "^⌥↩");
        assert_eq!(display("nonsense"), "nonsense");
    }

    #[test]
    fn every_default_parses() {
        for (action, chord) in DEFAULT_HOTKEYS {
            assert!(parse_chord(chord).is_some(), "{action}: {chord}");
            assert!(ACTION_LABELS.iter().any(|(a, _)| a == action), "{action} has no label");
        }
    }
}
