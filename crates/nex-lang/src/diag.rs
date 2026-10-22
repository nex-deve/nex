//! Compiler diagnostics and their human-readable rendering.

use crate::span::{SourceFile, Span};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
    pub label: Option<String>,
    pub notes: Vec<String>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic {
            message: message.into(),
            span,
            label: None,
            notes: Vec::new(),
        }
    }

    pub fn label(mut self, label: impl Into<String>) -> Diagnostic {
        self.label = Some(label.into());
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Diagnostic {
        self.notes.push(note.into());
        self
    }

    /// Render as `error: ...` followed by a source snippet with a caret.
    pub fn render(&self, src: &SourceFile) -> String {
        let (line, col) = src.line_col(self.span.start);
        let (end_line, end_col) = src.line_col(self.span.end);
        let width = if end_line == line && end_col > col {
            end_col - col
        } else {
            1
        };
        let mut out = String::new();
        let _ = writeln!(out, "error: {}", self.message);
        render_snippet(&mut out, src, line, col, width, self.label.as_deref());
        for note in &self.notes {
            let _ = writeln!(out, "  = note: {note}");
        }
        out
    }
}

/// Append a `--> file:line:col` header and the source line with a caret.
pub fn render_snippet(
    out: &mut String,
    src: &SourceFile,
    line: usize,
    col: usize,
    width: usize,
    label: Option<&str>,
) {
    let text = src.line_text(line);
    let num = line.to_string();
    let pad = " ".repeat(num.len());
    let _ = writeln!(out, "{pad}--> {}:{line}:{col}", src.name);
    let _ = writeln!(out, "{pad} |");
    let _ = writeln!(out, "{num} | {}", text.replace('\t', "    "));
    let indent: usize = text
        .chars()
        .take(col.saturating_sub(1))
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum();
    let _ = write!(out, "{pad} | {}{}", " ".repeat(indent), "^".repeat(width));
    if let Some(label) = label {
        let _ = write!(out, " {label}");
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_snippet_with_caret() {
        let src = SourceFile::new("main.nex", "app A {\n    say x\n}\n");
        let start = src.text.find('x').unwrap();
        let d = Diagnostic::error(
            "cannot find variable `x` in this scope",
            Span::new(start, start + 1),
        )
        .label("not found");
        let text = d.render(&src);
        assert!(text.contains("error: cannot find variable `x`"));
        assert!(text.contains("--> main.nex:2:9"));
        assert!(text.contains("2 |     say x"));
        assert!(text.contains("  |         ^ not found"));
    }
}
