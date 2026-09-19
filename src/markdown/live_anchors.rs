//! Block anchors kept out of sight while a block is edited in the Live view
//! (§3.2.1, anchors from §Fase 3): the raw editor shows `# Judul`, the file
//! keeps `# Judul ^8a6jg2`. `Hidden` pairs the text shown with the hidden
//! ` ^id` suffix of each line, puts the anchors back after an edit — lines
//! are matched by their unchanged head and tail, so inserted or removed
//! lines don't move an anchor onto the wrong text — and maps cursors
//! between the shown and the full text. Anchors in fences and anchor-only
//! lines (`^id` under a table) stay visible. Pure and egui-free.
//! Callers: `app::editor::live`.

use super::blocks::split_anchor;

/// A block's lines with their anchors hidden.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hidden {
    /// The text shown in the editor (lines joined by `\n`).
    pub visible: String,
    /// Per line of `visible`: the hidden suffix (` ^id`, with the one
    /// whitespace before it), appended back by `full`.
    suffixes: Vec<Option<String>>,
}

/// `line` split just before the whitespace that precedes its anchor:
/// `("# Judul", " ^8a6jg2")`. `None` when there is no anchor, or nothing
/// but the anchor on the line.
fn anchor_split(line: &str) -> Option<(&str, &str)> {
    split_anchor(line).1?;
    let caret = line.trim_end().rfind('^')?;
    let space = line[..caret].chars().next_back().filter(|c| c.is_whitespace())?;
    let cut = caret - space.len_utf8();
    let shown = &line[..cut];
    (!shown.trim().is_empty()).then_some((shown, &line[cut..]))
}

impl Hidden {
    /// Hides the anchors of `full` (lines joined by `\n`).
    pub fn new(full: &str) -> Self {
        let mut in_fence = false;
        let mut visible = Vec::new();
        let mut suffixes = Vec::new();
        for line in full.split('\n') {
            let t = line.trim_start();
            let fence = t.starts_with("```") || t.starts_with("~~~");
            if fence {
                in_fence = !in_fence;
            }
            match anchor_split(line).filter(|_| !fence && !in_fence) {
                Some((shown, suffix)) => {
                    visible.push(shown);
                    suffixes.push(Some(suffix.to_string()));
                }
                None => {
                    visible.push(line);
                    suffixes.push(None);
                }
            }
        }
        Hidden { visible: visible.join("\n"), suffixes }
    }

    /// The text with its anchors back in place.
    pub fn full(&self) -> String {
        self.visible
            .split('\n')
            .zip(&self.suffixes)
            .map(|(line, suffix)| format!("{line}{}", suffix.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `edited` (what the editor shows after an edit) with this text's
    /// anchors carried over. Unchanged lines at the start and the end keep
    /// their anchors; in the changed stretch between them lines are matched
    /// from its end, because an anchor belongs to the end of its line
    /// (Enter in the middle of `a b ^x` gives `a` and `b ^x`). Two anchors
    /// landing on one line: the later one wins. A line emptied of text
    /// drops its anchor.
    pub fn restore(&self, edited: &str) -> Hidden {
        let old: Vec<&str> = self.visible.split('\n').collect();
        let new: Vec<&str> = edited.split('\n').collect();
        let mut suffixes = vec![None; new.len()];
        let common = old.len().min(new.len());
        let head = (0..common).take_while(|&i| old[i] == new[i]).count();
        let tail = (0..common - head)
            .take_while(|&i| old[old.len() - 1 - i] == new[new.len() - 1 - i])
            .count();
        suffixes[..head].clone_from_slice(&self.suffixes[..head]);
        for i in 0..tail {
            suffixes[new.len() - 1 - i] = self.suffixes[old.len() - 1 - i].clone();
        }
        let (old_mid, new_mid) = (head..old.len() - tail, head..new.len() - tail);
        if !new_mid.is_empty() {
            for (k, i) in old_mid.clone().enumerate() {
                let from_end = old_mid.len() - 1 - k;
                let j = new_mid.end.saturating_sub(1 + from_end).max(new_mid.start);
                if self.suffixes[i].is_some() {
                    suffixes[j] = self.suffixes[i].clone();
                }
            }
        }
        for (suffix, line) in suffixes.iter_mut().zip(&new) {
            if line.trim().is_empty() {
                *suffix = None;
            }
        }
        Hidden { visible: edited.to_string(), suffixes }
    }

    /// Char index in `full` for the char index `cursor` in `visible`. The
    /// end of a line maps past its anchor, so Enter there leaves the anchor
    /// on the line it belongs to.
    pub fn to_full(&self, cursor: usize) -> usize {
        let (mut shown, mut full) = (0, 0);
        for (line, suffix) in self.visible.split('\n').zip(&self.suffixes) {
            let n = line.chars().count();
            let s = suffix.as_deref().map_or(0, |s| s.chars().count());
            if cursor <= shown + n {
                let col = cursor - shown;
                return full + col + if col == n { s } else { 0 };
            }
            shown += n + 1;
            full += n + s + 1;
        }
        full.saturating_sub(1)
    }

    /// Char index in `visible` for the char index `cursor` in `full`; a
    /// cursor inside an anchor lands at the end of its line.
    pub fn to_visible(&self, cursor: usize) -> usize {
        let (mut shown, mut full) = (0, 0);
        for (line, suffix) in self.visible.split('\n').zip(&self.suffixes) {
            let n = line.chars().count();
            let s = suffix.as_deref().map_or(0, |s| s.chars().count());
            if cursor <= full + n + s {
                return shown + (cursor - full).min(n);
            }
            shown += n + 1;
            full += n + s + 1;
        }
        shown.saturating_sub(1)
    }
}

/// Joins the full lines `upper` and `lower` into one, as Backspace at the
/// start of `lower` does. The result keeps one anchor at its end — the
/// upper line's, else the lower one's. Returns the line and the join point
/// (chars from its start).
pub fn join_lines(upper: &str, lower: &str) -> (String, usize) {
    let (up, up_anchor) = anchor_split(upper).unwrap_or((upper, ""));
    let (low, low_anchor) = anchor_split(lower).unwrap_or((lower, ""));
    let anchor = if up_anchor.is_empty() { low_anchor } else { up_anchor };
    (format!("{up}{low}{anchor}"), up.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_hide_and_come_back_unchanged() {
        for full in [
            "# 🚀 Proyek Startup AI ^8a6jg2",
            "> [!note] Data Pendukung ^49frkf\n> Lihat ![[data.csv]] ^2eff2p\n> dan `x.pdf` ^7ya3xe",
            "teks  ^abc\npolos\n",
            "```\nkode ^x\n```",
            "| a |\n| - |\n^tbl",
            "2^10 bukan anchor",
            "",
        ] {
            let h = Hidden::new(full);
            assert_eq!(h.full(), full);
            assert_eq!(h.restore(&h.visible), h, "an untouched text keeps every anchor");
        }
        assert_eq!(Hidden::new("# Judul ^8a6jg2").visible, "# Judul");
        assert_eq!(Hidden::new("teks  ^abc").visible, "teks ", "only the space before the anchor hides");
        assert_eq!(Hidden::new("```\nkode ^x\n```").visible, "```\nkode ^x\n```");
        assert_eq!(Hidden::new("| a |\n^tbl").visible, "| a |\n^tbl");
    }

    #[test]
    fn typing_keeps_the_anchor_at_the_end_of_its_line() {
        let h = Hidden::new("# Judul ^a1");
        assert_eq!(h.restore("# Judul baru").full(), "# Judul baru ^a1");
        // A trailing space typed at the end survives the next hide.
        let spaced = h.restore("# Judul ");
        assert_eq!(Hidden::new(&spaced.full()).visible, "# Judul ");
        // An anchor typed by hand stays text; the hidden one is kept.
        assert_eq!(h.restore("# Judul ^baru").full(), "# Judul ^baru ^a1");
    }

    #[test]
    fn inserted_and_removed_lines_do_not_shift_anchors() {
        let h = Hidden::new("> [!note] A ^c1\n> B ^c2\n> C ^c3");
        // Enter at the end of the first line of a callout.
        assert_eq!(h.restore("> [!note] A\n> \n> B\n> C").full(), "> [!note] A ^c1\n> \n> B ^c2\n> C ^c3");
        // Enter in the middle of a line: the anchor follows the line's end.
        assert_eq!(h.restore("> [!note] A\n> B1\n> B2\n> C").full(), "> [!note] A ^c1\n> B1\n> B2 ^c2\n> C ^c3");
        // A removed line takes its anchor with it.
        assert_eq!(h.restore("> [!note] A\n> C").full(), "> [!note] A ^c1\n> C ^c3");
        // A line emptied of text drops its anchor.
        assert_eq!(h.restore("> [!note] A\n\n> C").full(), "> [!note] A ^c1\n\n> C ^c3");
        // Two lines merged by a selection: one anchor, the later one.
        assert_eq!(h.restore("> [!note] A B\n> C").full(), "> [!note] A B ^c2\n> C ^c3");
    }

    #[test]
    fn cursors_map_between_shown_and_full_text() {
        let h = Hidden::new("ab ^x1\ncd");
        // visible "ab\ncd", full "ab ^x1\ncd"
        assert_eq!(h.to_full(1), 1);
        assert_eq!(h.to_full(2), 6, "end of line is past the anchor");
        assert_eq!(h.to_full(3), 7);
        assert_eq!(h.to_full(5), 9);
        assert_eq!(h.to_visible(4), 2, "inside the anchor clamps to the line end");
        assert_eq!(h.to_visible(7), 3);
        assert_eq!(h.to_visible(99), 5);
        for c in 0..=5 {
            assert_eq!(h.to_visible(h.to_full(c)), c);
        }
        // Chars, not bytes: "# 🚀 AI" is 6 chars, the anchor 8 more.
        assert_eq!(Hidden::new("# 🚀 AI ^8a6jg2").to_full(6), 14);
    }

    #[test]
    fn joined_lines_keep_one_anchor() {
        assert_eq!(join_lines("atas ^a", "bawah ^b"), ("atasbawah ^a".to_string(), 4));
        assert_eq!(join_lines("atas", "bawah ^b"), ("atasbawah ^b".to_string(), 4));
        assert_eq!(join_lines("satu", "dua"), ("satudua".to_string(), 4));
    }
}
