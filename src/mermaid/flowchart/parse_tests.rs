//! Unit tests for `flowchart::parse` (kept apart to keep `parse.rs` small).

use super::*;
use crate::mermaid::source::preprocess;

fn p(src: &str) -> (Flowchart, Vec<Diagnostic>) {
    parse(&preprocess(src))
}

#[test]
fn parses_shapes_and_labels() {
    let (fc, d) = p("flowchart LR\nA[Rect] --> B(Round)\nC([Stadium]) & D[[Sub]] & E[(DB)]\nF((Circle)) --- G>Odd]\nH{Rhombus} --> I{{Hex}}\nJ[/Lean/] --> K[\\Lean\\]\nL[/Trap\\] --> M[\\Inv/]\nN(((Dbl)))\nO@{ shape: doc, label: \"A doc\" }\n");
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(fc.dir, Dir::LR);
    let shape = |id: &str| fc.nodes[fc.node_index(id).unwrap()].shape;
    assert_eq!(shape("A"), Shape::Rect);
    assert_eq!(shape("B"), Shape::Round);
    assert_eq!(shape("C"), Shape::Stadium);
    assert_eq!(shape("D"), Shape::Subroutine);
    assert_eq!(shape("E"), Shape::Cylinder);
    assert_eq!(shape("F"), Shape::Circle);
    assert_eq!(shape("G"), Shape::Asymmetric);
    assert_eq!(shape("H"), Shape::Diamond);
    assert_eq!(shape("I"), Shape::Hexagon);
    assert_eq!(shape("J"), Shape::LeanRight);
    assert_eq!(shape("K"), Shape::LeanLeft);
    assert_eq!(shape("L"), Shape::Trapezoid);
    assert_eq!(shape("M"), Shape::InvTrapezoid);
    assert_eq!(shape("N"), Shape::DoubleCircle);
    assert_eq!(shape("O"), Shape::Document);
    assert_eq!(fc.nodes[fc.node_index("O").unwrap()].text(), "A doc");
    assert_eq!(fc.nodes[fc.node_index("A").unwrap()].text(), "Rect");
}

#[test]
fn parses_link_kinds_labels_and_lengths() {
    let (fc, d) = p("graph TD\nA-->B\nA --- C\nA -.-> D\nA ==> E\nA ~~~ F\nA <--> G\nA --o H\nA --x I\nA --->|long| J\nA -- text --> K\nA -. dotted .-> L\nA == thick ==> M\nA-->|\"quoted | pipe\"|N\n");
    assert!(d.iter().all(|x| !x.is_error()), "{d:?}");
    let e = |to: &str| fc.edges.iter().find(|e| e.to == to).unwrap().clone();
    assert_eq!((e("B").stroke, e("B").end, e("B").minlen), (LinkStroke::Normal, Some(Marker::Arrow), 1));
    assert_eq!(e("C").end, None);
    assert_eq!(e("D").stroke, LinkStroke::Dotted);
    assert_eq!(e("E").stroke, LinkStroke::Thick);
    assert_eq!(e("F").stroke, LinkStroke::Invisible);
    assert_eq!((e("G").start, e("G").end), (Some(Marker::Arrow), Some(Marker::Arrow)));
    assert_eq!(e("H").end, Some(Marker::Circle));
    assert_eq!(e("I").end, Some(Marker::Cross));
    assert_eq!((e("J").minlen, e("J").label.as_deref()), (2, Some("long")));
    assert_eq!(e("K").label.as_deref(), Some("text"));
    assert_eq!((e("L").stroke, e("L").label.as_deref()), (LinkStroke::Dotted, Some("dotted")));
    assert_eq!((e("M").stroke, e("M").label.as_deref()), (LinkStroke::Thick, Some("thick")));
}

#[test]
fn parses_chains_fanout_and_semicolons() {
    let (fc, d) = p("graph LR; A-->B-->C; D & E --> F & G\n");
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(fc.edges.len(), 2 + 4);
    assert_eq!(fc.nodes.len(), 7);
}

#[test]
fn parses_subgraphs_directions_and_styles() {
    let src = "flowchart TB\n  c1-->a2\n  subgraph one [First one]\n    direction LR\n    a1-->a2\n  end\n  subgraph two\n    b1-->b2\n    subgraph inner\n      x\n    end\n  end\n  one --> two\n  classDef hot fill:#f96,stroke:#333\n  class a1 hot\n  b1:::hot --> c1\n  style b2 fill:#bbf\n  linkStyle 0 stroke:#ff3,stroke-width:4px\n  click a1 href \"https://example.com\" \"Go\"\n";
    let (fc, d) = p(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(fc.subgraphs.len(), 3);
    let one = fc.subgraph_index("one").unwrap();
    assert_eq!(fc.subgraphs[one].title, "First one");
    assert_eq!(fc.subgraphs[one].dir, Some(Dir::LR));
    assert_eq!(fc.subgraphs[fc.subgraph_index("inner").unwrap()].parent, fc.subgraph_index("two"));
    let a2 = &fc.nodes[fc.node_index("a2").unwrap()];
    assert_eq!(a2.subgraph, Some(one), "a node listed in a subgraph belongs to it");
    let a1 = &fc.nodes[fc.node_index("a1").unwrap()];
    assert_eq!(a1.classes, vec!["hot"]);
    assert_eq!(a1.link.as_deref(), Some("https://example.com"));
    assert_eq!(a1.tooltip.as_deref(), Some("Go"));
    assert!(fc.nodes[fc.node_index("b1").unwrap()].classes.contains(&"hot".to_string()));
    assert!(fc.nodes[fc.node_index("b2").unwrap()].style.fill.is_some());
    assert!(fc.node_index("one").is_none(), "subgraph ids are not nodes");
    assert_eq!(fc.link_styles.len(), 1);
}

#[test]
fn reports_errors_with_positions_and_keeps_going() {
    let (fc, d) = p("flowchart LR\nA[unclosed --> B\nC --> D\nE -> F\nsubgraph s\n");
    let errors: Vec<&Diagnostic> = d.iter().filter(|x| x.is_error()).collect();
    assert!(errors.iter().any(|e| e.line == 2 && e.col == 2), "{d:?}");
    assert!(errors.iter().any(|e| e.line == 4), "{d:?}");
    assert!(errors.iter().any(|e| e.line == 5 && e.message.contains("never closed")), "{d:?}");
    assert!(fc.edges.iter().any(|e| e.from == "C" && e.to == "D"), "valid lines still parse");
}

#[test]
fn markdown_strings_and_entities() {
    let (fc, d) = p("flowchart LR\nA[\"`**Bold** line\nnext`\"] --> B[\"#quot;hi#quot; &amp; bye\"]\n");
    assert!(d.is_empty(), "{d:?}");
    let a = &fc.nodes[fc.node_index("A").unwrap()];
    assert!(a.markdown);
    assert_eq!(a.text(), "Bold line\nnext");
    assert_eq!(fc.nodes[fc.node_index("B").unwrap()].text(), "\"hi\" & bye");
}
