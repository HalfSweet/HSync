// SPDX-License-Identifier: Apache-2.0

//! From the IR's names to Rust's: fields, locals and functions in snake case, variants in
//! Pascal case, keywords escaped.

/// Rust's keywords, which an identifier may not be without `r#`.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "trait", "true", "type", "unsafe", "use", "where", "while",
    "abstract", "become", "box", "do", "final", "macro", "override", "priv", "try", "typeof",
    "unsized", "virtual", "yield", "gen",
];

/// Keywords that cannot be raw identifiers either.
const RESERVED: &[&str] = &["self", "Self", "super", "crate", "_"];

/// `camelCase` or `kebab-case` to `snake_case`.
pub fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let chars: Vec<char> = name.chars().collect();
    for (index, &c) in chars.iter().enumerate() {
        if c == '-' || c == ' ' {
            out.push('_');
        } else if c.is_ascii_uppercase() {
            let prev_lower = index > 0 && chars[index - 1].is_ascii_lowercase();
            let prev_digit = index > 0 && chars[index - 1].is_ascii_digit();
            let next_lower = chars.get(index + 1).is_some_and(|n| n.is_ascii_lowercase());
            let prev_upper = index > 0 && chars[index - 1].is_ascii_uppercase();
            if index > 0 && (prev_lower || prev_digit || (prev_upper && next_lower)) {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    escape(out)
}

/// `kebab-case`, `snake_case` or `camelCase` to `PascalCase`.
pub fn pascal(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = true;
    for c in name.chars() {
        if c == '-' || c == '_' || c == ' ' {
            upper = true;
        } else if upper {
            out.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// A variant identifier unique within its complete enum/union, retaining readable
/// PascalCase names whenever that spelling is legal and unambiguous. Encoded
/// names use a prefix that no readable candidate has, so user strings cannot
/// collide with the fallback namespace either. Serialization always uses the wire name.
pub fn variant_name<'a>(name: &str, names: impl IntoIterator<Item = &'a str>) -> String {
    let names: Vec<_> = names.into_iter().collect();
    let candidates: Vec<_> = names.iter().map(|name| pascal(name)).collect();
    let candidate = pascal(name);
    let legal = candidate.starts_with(|c: char| c.is_ascii_alphabetic())
        && candidate.chars().all(|c| c.is_ascii_alphanumeric())
        && !RESERVED.contains(&candidate.as_str())
        && !KEYWORDS.contains(&candidate.as_str());
    if legal
        && candidates
            .iter()
            .filter(|value| **value == candidate)
            .count()
            <= 1
    {
        return candidate;
    }
    let mut prefix = String::from("Wire");
    while candidates.iter().any(|name| name.starts_with(&prefix)) {
        prefix.push('X');
    }
    for byte in name.as_bytes() {
        use std::fmt::Write;
        let _ = write!(prefix, "{byte:02X}");
    }
    prefix
}

/// Escapes a keyword.
pub fn escape(name: String) -> String {
    if RESERVED.contains(&name.as_str()) {
        format!("{name}_")
    } else if KEYWORDS.contains(&name.as_str()) {
        format!("r#{name}")
    } else {
        name
    }
}

/// A Rust string literal.
pub fn string_literal(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A Rust string literal that is not a format string.
pub fn plain_literal(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_names_are_legal_unique_and_preserve_wire_identity() {
        let names = [
            "1.2",
            "2.0",
            "foo-bar",
            "foo_bar",
            "FooBar",
            "Self",
            "",
            "😀",
            "Wire666F6F",
        ];
        let rust: Vec<_> = names
            .iter()
            .map(|name| variant_name(name, names.iter().copied()))
            .collect();
        let unique: std::collections::HashSet<_> = rust.iter().collect();
        assert_eq!(unique.len(), names.len());
        for name in &rust {
            assert!(!name.is_empty());
            assert!(name.starts_with(|c: char| c.is_ascii_alphabetic()));
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric()));
            assert_ne!(name, "Self");
        }
        assert_eq!(
            variant_name("read-status", ["read-status", "write"]),
            "ReadStatus"
        );
    }

    #[test]
    fn names_convert() {
        assert_eq!(snake("periodTicks"), "period_ticks");
        assert_eq!(snake("onGap"), "on_gap");
        assert_eq!(snake("cs-release"), "cs_release");
        assert_eq!(snake("wordBits"), "word_bits");
        assert_eq!(snake("where"), "r#where");
        assert_eq!(snake("self"), "self_");
        assert_eq!(snake("MAX"), "max");
        assert_eq!(snake("isActive"), "is_active");
        assert_eq!(pascal("active-high"), "ActiveHigh");
        assert_eq!(pascal("read-status-2"), "ReadStatus2");
        assert_eq!(pascal("enter-4b-mode"), "Enter4bMode");
        assert_eq!(pascal("cycle"), "Cycle");
        assert_eq!(pascal("SpiEvent"), "SpiEvent");
        assert_eq!(string_literal("a {b} \"c\""), "\"a {{b}} \\\"c\\\"\"");
    }
}
