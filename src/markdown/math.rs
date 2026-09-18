//! LaTeX math for notes (§Fase 1.8 "math dengan fallback"): egui has no
//! TeX engine, so `$…$` / `$$…$$` are rendered as Unicode text — Greek
//! letters, operators, super/subscripts, fractions, roots — which reads
//! well for the everyday formulas a note holds and degrades to the raw
//! source for anything exotic. Pure string logic; `markdown::renderer`
//! feeds it to `egui_commonmark`'s `render_math_fn`. Callers:
//! `markdown::renderer`, `markdown::highlight`.

/// `\command` → replacement (letters, operators, arrows, spacing).
const COMMANDS: &[(&str, &str)] = &[
    ("alpha", "α"), ("beta", "β"), ("gamma", "γ"), ("delta", "δ"), ("epsilon", "ε"),
    ("varepsilon", "ε"), ("zeta", "ζ"), ("eta", "η"), ("theta", "θ"), ("vartheta", "ϑ"),
    ("iota", "ι"), ("kappa", "κ"), ("lambda", "λ"), ("mu", "μ"), ("nu", "ν"), ("xi", "ξ"),
    ("pi", "π"), ("rho", "ρ"), ("sigma", "σ"), ("tau", "τ"), ("upsilon", "υ"), ("phi", "φ"),
    ("varphi", "ϕ"), ("chi", "χ"), ("psi", "ψ"), ("omega", "ω"),
    ("Gamma", "Γ"), ("Delta", "Δ"), ("Theta", "Θ"), ("Lambda", "Λ"), ("Xi", "Ξ"), ("Pi", "Π"),
    ("Sigma", "Σ"), ("Phi", "Φ"), ("Psi", "Ψ"), ("Omega", "Ω"),
    ("times", "×"), ("cdot", "·"), ("div", "÷"), ("pm", "±"), ("mp", "∓"), ("ast", "∗"),
    ("le", "≤"), ("leq", "≤"), ("ge", "≥"), ("geq", "≥"), ("ne", "≠"), ("neq", "≠"),
    ("approx", "≈"), ("equiv", "≡"), ("sim", "∼"), ("simeq", "≃"), ("propto", "∝"),
    ("ll", "≪"), ("gg", "≫"), ("subset", "⊂"), ("subseteq", "⊆"), ("supset", "⊃"),
    ("in", "∈"), ("notin", "∉"), ("cup", "∪"), ("cap", "∩"), ("setminus", "∖"),
    ("emptyset", "∅"), ("varnothing", "∅"), ("forall", "∀"), ("exists", "∃"), ("neg", "¬"),
    ("land", "∧"), ("lor", "∨"), ("wedge", "∧"), ("vee", "∨"), ("oplus", "⊕"), ("otimes", "⊗"),
    ("to", "→"), ("rightarrow", "→"), ("leftarrow", "←"), ("Rightarrow", "⇒"),
    ("Leftarrow", "⇐"), ("leftrightarrow", "↔"), ("Leftrightarrow", "⇔"), ("mapsto", "↦"),
    ("uparrow", "↑"), ("downarrow", "↓"),
    ("infty", "∞"), ("partial", "∂"), ("nabla", "∇"), ("sum", "∑"), ("prod", "∏"),
    ("int", "∫"), ("iint", "∬"), ("oint", "∮"), ("lim", "lim"), ("log", "log"), ("ln", "ln"),
    ("exp", "exp"), ("sin", "sin"), ("cos", "cos"), ("tan", "tan"), ("min", "min"),
    ("max", "max"), ("det", "det"), ("dim", "dim"), ("deg", "°"), ("circ", "∘"),
    ("angle", "∠"), ("perp", "⊥"), ("parallel", "∥"), ("ldots", "…"), ("cdots", "⋯"),
    ("vdots", "⋮"), ("dots", "…"), ("prime", "′"), ("hbar", "ℏ"), ("ell", "ℓ"),
    ("Re", "ℜ"), ("Im", "ℑ"), ("aleph", "ℵ"), ("mathbb{R}", "ℝ"), ("mathbb{N}", "ℕ"),
    ("mathbb{Z}", "ℤ"), ("mathbb{Q}", "ℚ"), ("mathbb{C}", "ℂ"), ("langle", "⟨"),
    ("rangle", "⟩"), ("lfloor", "⌊"), ("rfloor", "⌋"), ("lceil", "⌈"), ("rceil", "⌉"),
    ("quad", "  "), ("qquad", "    "), (",", " "), (";", " "), (":", " "), ("!", ""),
    ("{", "{"), ("}", "}"), ("%", "%"), ("&", "&"), ("_", "_"), ("#", "#"), ("$", "$"),
    ("backslash", "\\"), ("|", "‖"), ("vert", "|"), ("mid", "|"),
];

fn superscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴', '5' => '⁵', '6' => '⁶',
        '7' => '⁷', '8' => '⁸', '9' => '⁹', '+' => '⁺', '-' => '⁻', '=' => '⁼', '(' => '⁽',
        ')' => '⁾', 'n' => 'ⁿ', 'i' => 'ⁱ', 'a' => 'ᵃ', 'b' => 'ᵇ', 'c' => 'ᶜ', 'd' => 'ᵈ',
        'e' => 'ᵉ', 'f' => 'ᶠ', 'g' => 'ᵍ', 'h' => 'ʰ', 'j' => 'ʲ', 'k' => 'ᵏ', 'l' => 'ˡ',
        'm' => 'ᵐ', 'o' => 'ᵒ', 'p' => 'ᵖ', 'r' => 'ʳ', 's' => 'ˢ', 't' => 'ᵗ', 'u' => 'ᵘ',
        'v' => 'ᵛ', 'w' => 'ʷ', 'x' => 'ˣ', 'y' => 'ʸ', 'z' => 'ᶻ', 'T' => 'ᵀ', ' ' => ' ',
        _ => return None,
    })
}

fn subscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄', '5' => '₅', '6' => '₆',
        '7' => '₇', '8' => '₈', '9' => '₉', '+' => '₊', '-' => '₋', '=' => '₌', '(' => '₍',
        ')' => '₎', 'a' => 'ₐ', 'e' => 'ₑ', 'h' => 'ₕ', 'i' => 'ᵢ', 'j' => 'ⱼ', 'k' => 'ₖ',
        'l' => 'ₗ', 'm' => 'ₘ', 'n' => 'ₙ', 'o' => 'ₒ', 'p' => 'ₚ', 'r' => 'ᵣ', 's' => 'ₛ',
        't' => 'ₜ', 'u' => 'ᵤ', 'v' => 'ᵥ', 'x' => 'ₓ', ' ' => ' ',
        _ => return None,
    })
}

/// Converts LaTeX `tex` to readable Unicode. Never fails: unknown
/// commands keep their name (without the backslash).
pub fn to_unicode(tex: &str) -> String {
    let chars: Vec<char> = tex.chars().collect();
    let mut i = 0;
    let out = parse_seq(&chars, &mut i, None);
    collapse_spaces(&out)
}

fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(c);
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Parses until `until` (a closing brace) or the end.
fn parse_seq(chars: &[char], i: &mut usize, until: Option<char>) -> String {
    let mut out = String::new();
    while *i < chars.len() {
        let c = chars[*i];
        if Some(c) == until {
            *i += 1;
            return out;
        }
        match c {
            '\\' => {
                *i += 1;
                let cmd = read_command(chars, i);
                out.push_str(&expand_command(&cmd, chars, i));
            }
            '{' => {
                *i += 1;
                out.push_str(&parse_seq(chars, i, Some('}')));
            }
            '^' | '_' => {
                *i += 1;
                let arg = read_argument(chars, i);
                let map: fn(char) -> Option<char> = if c == '^' { superscript } else { subscript };
                let converted: Option<String> = arg.chars().map(map).collect();
                match converted {
                    Some(s) if !arg.is_empty() => out.push_str(&s),
                    _ => {
                        out.push(if c == '^' { '^' } else { '_' });
                        if arg.chars().count() > 1 {
                            out.push_str(&format!("({arg})"));
                        } else {
                            out.push_str(&arg);
                        }
                    }
                }
            }
            '&' => {
                out.push(' ');
                *i += 1;
            }
            '~' => {
                out.push(' ');
                *i += 1;
            }
            _ => {
                out.push(c);
                *i += 1;
            }
        }
    }
    out
}

/// Reads a command name after a backslash: letters, or one symbol char.
fn read_command(chars: &[char], i: &mut usize) -> String {
    let mut name = String::new();
    if *i < chars.len() && !chars[*i].is_alphabetic() {
        name.push(chars[*i]);
        *i += 1;
        return name;
    }
    while *i < chars.len() && chars[*i].is_alphabetic() {
        name.push(chars[*i]);
        *i += 1;
    }
    name
}

/// A braced group, or a single token (one char or one `\command`).
fn read_argument(chars: &[char], i: &mut usize) -> String {
    while *i < chars.len() && chars[*i] == ' ' {
        *i += 1;
    }
    if *i >= chars.len() {
        return String::new();
    }
    if chars[*i] == '{' {
        *i += 1;
        return parse_seq(chars, i, Some('}'));
    }
    if chars[*i] == '\\' {
        *i += 1;
        let cmd = read_command(chars, i);
        return expand_command(&cmd, chars, i);
    }
    let c = chars[*i];
    *i += 1;
    c.to_string()
}

fn expand_command(cmd: &str, chars: &[char], i: &mut usize) -> String {
    match cmd {
        "frac" | "dfrac" | "tfrac" => {
            let a = read_argument(chars, i);
            let b = read_argument(chars, i);
            format!("{}/{}", group(&a), group(&b))
        }
        "sqrt" => {
            // Optional root index `\sqrt[n]{x}`.
            let index = if *i < chars.len() && chars[*i] == '[' {
                *i += 1;
                let s = parse_seq(chars, i, Some(']'));
                s.chars().filter_map(superscript).collect::<String>()
            } else {
                String::new()
            };
            let a = read_argument(chars, i);
            format!("{index}√{}", if a.chars().count() > 1 { format!("({a})") } else { a })
        }
        "text" | "mathrm" | "mathbf" | "mathit" | "textbf" | "textit" | "operatorname"
        | "mathcal" | "mathsf" | "boldsymbol" | "vec" | "hat" | "bar" | "tilde" | "dot"
        | "overline" | "underline" => {
            let a = read_argument(chars, i);
            match cmd {
                "vec" => format!("{a}⃗"),
                "hat" => format!("{a}̂"),
                "bar" | "overline" => format!("{a}̄"),
                "tilde" => format!("{a}̃"),
                "dot" => format!("{a}̇"),
                _ => a,
            }
        }
        "mathbb" => {
            let a = read_argument(chars, i);
            COMMANDS
                .iter()
                .find(|(k, _)| *k == format!("mathbb{{{a}}}"))
                .map(|(_, v)| v.to_string())
                .unwrap_or(a)
        }
        "left" | "right" | "displaystyle" | "textstyle" | "limits" | "nolimits" => String::new(),
        "begin" | "end" => {
            let _ = read_argument(chars, i);
            String::new()
        }
        "\\" => "\n".to_string(),
        other => COMMANDS
            .iter()
            .find(|(k, _)| *k == other)
            .map(|(_, v)| v.to_string())
            .unwrap_or_else(|| other.to_string()),
    }
}

/// Wraps multi-token operands in parentheses so `\frac{a+b}{2}` reads
/// `(a+b)/2`.
fn group(s: &str) -> String {
    let simple = s.chars().count() <= 1
        || s.chars().all(|c| c.is_alphanumeric() || "⁰¹²³⁴⁵⁶⁷⁸⁹₀₁₂₃₄₅₆₇₈₉".contains(c));
    if simple { s.to_string() } else { format!("({s})") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_formulas() {
        assert_eq!(to_unicode("E = mc^2"), "E = mc²");
        assert_eq!(to_unicode(r"\alpha + \beta \le \gamma"), "α + β ≤ γ");
        assert_eq!(to_unicode(r"\frac{a+b}{2}"), "(a+b)/2");
        assert_eq!(to_unicode(r"\sqrt{x^2 + y^2}"), "√(x² + y²)");
        assert_eq!(to_unicode(r"\sum_{i=1}^{n} x_i"), "∑ᵢ₌₁ⁿ xᵢ");
        assert_eq!(to_unicode(r"x \in \mathbb{R}, \forall x \ne 0"), "x ∈ ℝ, ∀ x ≠ 0");
        assert_eq!(to_unicode(r"\text{harga} \times 2"), "harga × 2");
        assert_eq!(to_unicode(r"\lim_{x \to \infty} f(x)"), "lim_(x → ∞) f(x)");
    }

    #[test]
    fn unknown_commands_and_broken_input_degrade_gracefully() {
        assert_eq!(to_unicode(r"\foo{x}"), "foox");
        assert_eq!(to_unicode(r"a^{"), "a^");
        assert_eq!(to_unicode(r"\frac{1}"), "1/");
        assert_eq!(to_unicode(""), "");
        assert_eq!(to_unicode(r"\left( \frac{1}{2} \right)"), "( 1/2 )");
    }
}
