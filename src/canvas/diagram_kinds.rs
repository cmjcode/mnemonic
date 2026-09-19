//! Diagram vocabulary shared by the canvas and Mermaid (§3.9.3): ER entity
//! attributes, crow's-foot cardinalities, class relations and the
//! per-connector metadata (`ConnectorMeta`) that lets a canvas round-trip
//! to `erDiagram` / `classDiagram` / `flowchart`. Also the plain-text
//! editing form of entity and class boxes (one line per attribute,
//! Mermaid-like), so they edit in the same inline text box as notes.
//! Pure data, no egui. Callers: `canvas::element`, `canvas::painter`,
//! `canvas::jsoncanvas`, `canvas::mermaid_export`, `canvas::mermaid_import`.

use serde::{Deserialize, Serialize};

/// One `type name PK,FK "comment"` row of an ER entity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityAttr {
    pub ty: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub keys: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

impl EntityAttr {
    /// `type name KEYS "comment"` (the Mermaid attribute syntax).
    pub fn to_line(&self) -> String {
        let mut s = format!("{} {}", self.ty, self.name);
        if !self.keys.is_empty() {
            s.push(' ');
            s.push_str(&self.keys);
        }
        if !self.comment.is_empty() {
            s.push_str(&format!(" \"{}\"", self.comment.replace('"', "'")));
        }
        s
    }

    /// Parses `type name [PK, FK] ["comment"]`; `None` for a blank line.
    /// A lone word is taken as the name with type `string`.
    pub fn parse_line(line: &str) -> Option<EntityAttr> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let (head, comment) = match line.find('"') {
            Some(q) => (&line[..q], line[q + 1..].trim_end_matches('"').to_string()),
            None => (line, String::new()),
        };
        let mut words = head.split_whitespace();
        let first = words.next()?.to_string();
        let Some(second) = words.next() else {
            return Some(EntityAttr { ty: "string".into(), name: first, keys: String::new(), comment });
        };
        let keys: Vec<String> = words
            .flat_map(|w| w.split(','))
            .map(|k| k.trim().to_ascii_uppercase())
            .filter(|k| matches!(k.as_str(), "PK" | "FK" | "UK"))
            .collect();
        Some(EntityAttr { ty: first, name: second.to_string(), keys: keys.join(", "), comment })
    }
}

/// Crow's-foot end of an ER relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErCardinality {
    ExactlyOne,
    ZeroOrOne,
    OneOrMore,
    ZeroOrMore,
}

impl ErCardinality {
    /// Mermaid token on the left side of `--` (`||`, `|o`, `}|`, `}o`).
    pub fn left_token(self) -> &'static str {
        match self {
            ErCardinality::ExactlyOne => "||",
            ErCardinality::ZeroOrOne => "|o",
            ErCardinality::OneOrMore => "}|",
            ErCardinality::ZeroOrMore => "}o",
        }
    }

    /// Mermaid token on the right side of `--` (`||`, `o|`, `|{`, `o{`).
    pub fn right_token(self) -> &'static str {
        match self {
            ErCardinality::ExactlyOne => "||",
            ErCardinality::ZeroOrOne => "o|",
            ErCardinality::OneOrMore => "|{",
            ErCardinality::ZeroOrMore => "o{",
        }
    }

    pub const ALL: [ErCardinality; 4] = [
        ErCardinality::ExactlyOne,
        ErCardinality::ZeroOrOne,
        ErCardinality::OneOrMore,
        ErCardinality::ZeroOrMore,
    ];
}

/// UML relation between two class boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassRelKind {
    Inheritance,
    Composition,
    Aggregation,
    Association,
    Dependency,
    Realization,
    Link,
}

impl ClassRelKind {
    /// Mermaid arrow written `From <arrow> To` with the marker at `From`
    /// for inheritance/composition/aggregation (`Animal <|-- Dog`).
    pub fn arrow(self) -> &'static str {
        match self {
            ClassRelKind::Inheritance => "<|--",
            ClassRelKind::Composition => "*--",
            ClassRelKind::Aggregation => "o--",
            ClassRelKind::Association => "-->",
            ClassRelKind::Dependency => "..>",
            ClassRelKind::Realization => "..|>",
            ClassRelKind::Link => "--",
        }
    }

    pub fn is_dashed(self) -> bool {
        matches!(self, ClassRelKind::Dependency | ClassRelKind::Realization)
    }

    pub const ALL: [ClassRelKind; 7] = [
        ClassRelKind::Inheritance,
        ClassRelKind::Composition,
        ClassRelKind::Aggregation,
        ClassRelKind::Association,
        ClassRelKind::Dependency,
        ClassRelKind::Realization,
        ClassRelKind::Link,
    ];
}

/// Diagram meaning of a connector (drawn as markers, exported as the
/// matching Mermaid relation).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EdgeRelation {
    Er { from: ErCardinality, to: ErCardinality, identifying: bool },
    Class {
        kind: ClassRelKind,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        card_from: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        card_to: String,
    },
}

/// Extra connector data. `outline` marks the parent → child edges the
/// section outline maintains (re-derived from heading nesting; never
/// user-drawn).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectorMeta {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outline: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dashed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<EdgeRelation>,
}

impl ConnectorMeta {
    pub fn is_default(&self) -> bool {
        *self == ConnectorMeta::default()
    }

    pub fn outline() -> Self {
        ConnectorMeta { outline: true, ..Default::default() }
    }
}

/// Editing form of an entity: name on the first line, one attribute per line.
pub fn entity_to_text(name: &str, attrs: &[EntityAttr]) -> String {
    let mut s = name.to_string();
    for a in attrs {
        s.push('\n');
        s.push_str(&a.to_line());
    }
    s
}

/// Inverse of [`entity_to_text`].
pub fn entity_from_text(text: &str) -> (String, Vec<EntityAttr>) {
    let mut lines = text.lines();
    let name = lines.next().unwrap_or("").trim().to_string();
    (name, lines.filter_map(EntityAttr::parse_line).collect())
}

/// Editing form of a class: `<<annotation>>` (optional) then the name, then
/// members; lines with `(` are methods.
pub fn class_to_text(name: &str, annotation: &str, attributes: &[String], methods: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    if !annotation.is_empty() {
        out.push(format!("<<{annotation}>>"));
    }
    out.push(name.to_string());
    out.extend(attributes.iter().cloned());
    out.extend(methods.iter().cloned());
    out.join("\n")
}

/// Inverse of [`class_to_text`]: `(name, annotation, attributes, methods)`.
pub fn class_from_text(text: &str) -> (String, String, Vec<String>, Vec<String>) {
    let mut annotation = String::new();
    let mut name = String::new();
    let mut attributes = Vec::new();
    let mut methods = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if name.is_empty() {
            if let Some(a) = line.strip_prefix("<<").and_then(|l| l.strip_suffix(">>")) {
                annotation = a.trim().to_string();
            } else {
                name = line.to_string();
            }
        } else if line.contains('(') {
            methods.push(line.to_string());
        } else {
            attributes.push(line.to_string());
        }
    }
    (name, annotation, attributes, methods)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_text_round_trip() {
        let text = "CUSTOMER\nstring id PK\nint age\nstring email UK \"login\"";
        let (name, attrs) = entity_from_text(text);
        assert_eq!(name, "CUSTOMER");
        assert_eq!(attrs.len(), 3);
        assert_eq!(attrs[0].keys, "PK");
        assert_eq!(attrs[2].comment, "login");
        assert_eq!(entity_to_text(&name, &attrs), text);
        assert_eq!(EntityAttr::parse_line("nama").unwrap().ty, "string");
        assert_eq!(EntityAttr::parse_line("int a pk,fk").unwrap().keys, "PK, FK");
    }

    #[test]
    fn class_text_round_trip() {
        let text = "<<interface>>\nAnimal\n+String name\n+speak() void";
        let (name, ann, attrs, methods) = class_from_text(text);
        assert_eq!((name.as_str(), ann.as_str()), ("Animal", "interface"));
        assert_eq!(attrs, vec!["+String name"]);
        assert_eq!(methods, vec!["+speak() void"]);
        assert_eq!(class_to_text(&name, &ann, &attrs, &methods), text);
    }

    #[test]
    fn meta_default_is_skipped() {
        assert!(ConnectorMeta::default().is_default());
        let json = serde_json::to_string(&ConnectorMeta {
            relation: Some(EdgeRelation::Er {
                from: ErCardinality::ExactlyOne,
                to: ErCardinality::ZeroOrMore,
                identifying: true,
            }),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(json, r#"{"relation":{"type":"er","from":"exactly_one","to":"zero_or_more","identifying":true}}"#);
    }
}
