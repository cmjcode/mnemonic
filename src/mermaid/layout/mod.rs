//! Layout engines shared by the Mermaid diagram types (§3.7.3). Four
//! families cover every diagram: `layered` (Sugiyama — flowchart, class,
//! state, ER, requirement, C4), and per-type linear, tree and chart
//! layouts that live next to their diagram because they are small.
//! Callers: `mermaid::flowchart` and the other graph-shaped diagrams.

pub mod layered;

/// Flow direction of a graph layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dir {
    #[default]
    TB,
    BT,
    LR,
    RL,
}

impl Dir {
    /// `TB`/`TD`/`BT`/`LR`/`RL` (also `>`/`<`/`^`/`v` as Mermaid allows).
    pub fn parse(s: &str) -> Option<Dir> {
        match s.trim().to_ascii_uppercase().as_str() {
            "TB" | "TD" | "V" => Some(Dir::TB),
            "BT" | "^" => Some(Dir::BT),
            "LR" | ">" => Some(Dir::LR),
            "RL" | "<" => Some(Dir::RL),
            _ => None,
        }
    }

    pub fn is_horizontal(self) -> bool {
        matches!(self, Dir::LR | Dir::RL)
    }
}
