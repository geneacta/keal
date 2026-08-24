//! Source locations, the source map, and diagnostic rendering.

use std::fmt;
use std::path::{Path, PathBuf};

/// A location in a source file. `file` indexes into [`Sources`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Span {
    pub file: u32,
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub fn new(file: u32, line: u32, col: u32) -> Span {
        Span { file, line, col }
    }
}

/// A user-facing error tied to a source location.
#[derive(Clone, Debug)]
pub struct Diag {
    pub msg: String,
    pub span: Span,
    /// Optional second line printed under the caret, e.g. a hint.
    pub note: Option<String>,
}

impl Diag {
    pub fn new(span: Span, msg: impl Into<String>) -> Diag {
        Diag { msg: msg.into(), span, note: None }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Diag {
        self.note = Some(note.into());
        self
    }
}

/// Every file the compiler has loaded, so diagnostics can quote source lines.
#[derive(Default)]
pub struct Sources {
    files: Vec<SourceFile>,
}

pub struct SourceFile {
    pub path: PathBuf,
    pub text: String,
}

impl Sources {
    pub fn new() -> Sources {
        Sources { files: Vec::new() }
    }

    pub fn add(&mut self, path: impl AsRef<Path>, text: String) -> u32 {
        let id = self.files.len() as u32;
        self.files.push(SourceFile { path: path.as_ref().to_path_buf(), text });
        id
    }

    pub fn get(&self, id: u32) -> Option<&SourceFile> {
        self.files.get(id as usize)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn path(&self, id: u32) -> String {
        self.get(id).map(|f| f.path.display().to_string()).unwrap_or_else(|| "<unknown>".into())
    }

    fn line_text(&self, id: u32, line: u32) -> Option<&str> {
        self.get(id)?.text.lines().nth(line.saturating_sub(1) as usize)
    }

    /// Renders a diagnostic the way rustc does: header, quoted line, caret.
    pub fn render(&self, kind: &str, d: &Diag) -> String {
        let mut out = String::new();
        out.push_str(&format!("{}: {}\n", kind, d.msg));
        out.push_str(&format!(
            "  --> {}:{}:{}\n",
            self.path(d.span.file),
            d.span.line,
            d.span.col
        ));
        if let Some(line) = self.line_text(d.span.file, d.span.line) {
            let num = d.span.line.to_string();
            let pad = " ".repeat(num.len());
            out.push_str(&format!("{} |\n", pad));
            out.push_str(&format!("{} | {}\n", num, line.replace('\t', "    ")));
            // Account for tabs expanded to 4 spaces when placing the caret.
            let prefix: String = line
                .chars()
                .take(d.span.col.saturating_sub(1) as usize)
                .map(|c| if c == '\t' { "    ".to_string() } else { " ".to_string() })
                .collect();
            out.push_str(&format!("{} | {}^\n", pad, prefix));
        }
        if let Some(note) = &d.note {
            out.push_str(&format!("  = note: {}\n", note));
        }
        out
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}
