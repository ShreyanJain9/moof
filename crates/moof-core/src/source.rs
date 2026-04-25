// Source preservation.
//
// Every closure compiled from moof source gets a `ClosureSource`
// attached — the text the user typed, plus where it came from. Moof
// code can read it back via `[closure source]` and `[closure origin]`,
// which powers inspectors, source browsers, and the eventual "edit
// handler in-image, save to file" authoring gesture.
//
// Storage: sources live on the Heap, keyed by `code_idx` (an index
// into VM.closure_descs). Multiple closure *instances* with different
// captures share a single source record — the source text is an
// artifact of the compiled code, not the capture environment.

/// The source text that produced a closure desc, plus its provenance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClosureSource {
    /// The source text of the *top-level form* that produced this
    /// closure. A `(defmethod Foo bar: ...)` form produces the handler
    /// closure; the whole defmethod is that closure's source. Inner
    /// closures (lambdas nested in a method) inherit the outer form's
    /// source today — finer-grained per-lambda source requires
    /// parser position tracking, a future refinement.
    pub text: String,

    /// Where the source came from. Typically a path like
    /// `"lib/data/option.moof"`, or `"<repl>"` for interactive input,
    /// or `"<eval>"` for runtime-eval'd strings.
    pub origin: SourceOrigin,
}

/// Provenance of a piece of source.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceOrigin {
    /// Label: a file path, or `<repl>`, or `<eval>`, or other.
    pub label: String,

    /// Byte offset where this form starts in the labeled source.
    /// Useful for "jump to definition" and round-trip editing.
    pub byte_start: usize,

    /// Byte offset one past the end.
    pub byte_end: usize,
}

impl SourceOrigin {
    pub fn anon(label: impl Into<String>) -> Self {
        SourceOrigin { label: label.into(), byte_start: 0, byte_end: 0 }
    }
}

/// Per-form source location. The parser records one of these for
/// every cons cell (and other heap-allocated form value) it
/// produces, keyed by the value's heap id, into `Heap::form_locations`.
/// The `[v __form-text]` primitive looks the entry up and slices
/// the source. In-memory only — not persisted across image
/// loads (sources are re-parsed when files reload).
#[derive(Clone, Debug)]
pub struct FormLoc {
    /// The full source text the form was parsed from. Shared via
    /// Arc — every form parsed in one parse-session points at the
    /// same string.
    pub source: std::sync::Arc<str>,
    pub byte_start: u32,
    pub byte_end: u32,
}

impl FormLoc {
    pub fn slice(&self) -> &str {
        &self.source[self.byte_start as usize .. self.byte_end as usize]
    }
}

/// Split a moof source string into its top-level forms, each paired
/// with the byte range it occupied in the original input.
///
/// Handles the full moof surface: all three bracket species
/// (`( )`, `[ ]`, `{ }`), double-quoted strings with `\` escapes,
/// `;` line comments, and bare atoms at top level.
///
/// Malformed input (unbalanced brackets, unterminated strings) is
/// best-effort: we return what we could extract before giving up.
/// The full parser will re-error on the form anyway.
pub fn split_top_level_forms(source: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        // skip whitespace + line comments
        loop {
            if i >= bytes.len() { break; }
            let c = bytes[i];
            if c.is_ascii_whitespace() {
                i += 1;
            } else if c == b';' {
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            } else {
                break;
            }
        }
        if i >= bytes.len() { break; }

        let start = i;
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escape = false;
        let mut saw_bracket = false;

        while i < bytes.len() {
            let c = bytes[i];

            if escape {
                escape = false;
                i += 1;
                continue;
            }
            if in_string {
                if c == b'\\' { escape = true; }
                else if c == b'"' { in_string = false; }
                i += 1;
                continue;
            }

            match c {
                b'"' => { in_string = true; i += 1; }
                b';' => {
                    // end-of-line comment; if we're inside a bracketed
                    // form, skip to end of line and continue. if we're
                    // on a bare atom, the atom ends here.
                    if depth == 0 && !saw_bracket { break; }
                    while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
                }
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    saw_bracket = true;
                    i += 1;
                }
                b')' | b']' | b'}' => {
                    depth -= 1;
                    i += 1;
                    if depth <= 0 && saw_bracket { break; }
                }
                c if c.is_ascii_whitespace() => {
                    if depth == 0 && !saw_bracket { break; }  // atom terminator
                    i += 1;
                }
                _ => i += 1,
            }
        }

        if i > start {
            // drop trailing whitespace inside the captured range (the
            // token-terminator whitespace got eaten above)
            let end = i;
            let text = std::str::from_utf8(&bytes[start..end])
                .unwrap_or("")
                .to_string();
            out.push((text, start..end));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_paren_form() {
        let src = "(def x 42)";
        let forms = split_top_level_forms(src);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].0, "(def x 42)");
        assert_eq!(forms[0].1, 0..10);
    }

    #[test]
    fn multiple_forms() {
        let src = "(def x 1)\n(def y 2)";
        let forms = split_top_level_forms(src);
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].0, "(def x 1)");
        assert_eq!(forms[1].0, "(def y 2)");
    }

    #[test]
    fn nested_brackets() {
        let src = "(foo [bar {baz: 1}])";
        let forms = split_top_level_forms(src);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].0, "(foo [bar {baz: 1}])");
    }

    #[test]
    fn string_with_parens_inside() {
        let src = r#"(def x "hello (world)")"#;
        let forms = split_top_level_forms(src);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].0, r#"(def x "hello (world)")"#);
    }

    #[test]
    fn line_comments_skipped() {
        let src = "; this is a comment\n(def x 1)";
        let forms = split_top_level_forms(src);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].0, "(def x 1)");
    }

    #[test]
    fn inline_comment_inside_form() {
        let src = "(def x\n  ; commentary\n  42)";
        let forms = split_top_level_forms(src);
        assert_eq!(forms.len(), 1);
    }

    #[test]
    fn bare_atom() {
        let src = "42";
        let forms = split_top_level_forms(src);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].0, "42");
    }

    #[test]
    fn atom_then_form() {
        let src = "foo\n(def x 1)";
        let forms = split_top_level_forms(src);
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].0, "foo");
        assert_eq!(forms[1].0, "(def x 1)");
    }

    #[test]
    fn escaped_quote_in_string() {
        let src = r#"(def x "a\"b") (def y 1)"#;
        let forms = split_top_level_forms(src);
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].0, r#"(def x "a\"b")"#);
    }

    #[test]
    fn empty_input() {
        assert!(split_top_level_forms("").is_empty());
        assert!(split_top_level_forms("   \n\n  ").is_empty());
        assert!(split_top_level_forms(";; just a comment").is_empty());
    }
}
