//! Parser diagnostics with source positions (§3.7.2): every Mermaid parser
//! reports problems as data — it never panics and never gives up on the
//! whole diagram because of one bad line — so the editor can underline the
//! exact spot and agents can fix their output from `mnemonic-cli diagram
//! validate`. Callers: every `mermaid::*` parser, `api::diagram`.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// One problem in a diagram source. `line`/`col` are 1-based and relative
/// to the diagram source (the text inside the fence), `col` in chars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub line: usize,
    pub col: usize,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    pub fn error(line: usize, col: usize, message: impl Into<String>) -> Diagnostic {
        Diagnostic { line, col, severity: Severity::Error, message: message.into() }
    }

    pub fn warning(line: usize, col: usize, message: impl Into<String>) -> Diagnostic {
        Diagnostic { line, col, severity: Severity::Warning, message: message.into() }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{}:{}: {level}: {}", self.line, self.col, self.message)
    }
}
