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

/// A path as a diagnostic spells it: always with `/`, whatever the platform
/// renders. Two compilers that must agree byte for byte cannot disagree
/// about a separator, and a message is compared whole — the path inside its
/// sentence as much as the location above it.
pub fn shown(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Every file the compiler has loaded, so diagnostics can quote source lines.
///
/// Cloned once, by the C backend: a compiled program carries the lines it
/// might have to quote, and the backend needs them while it emits.
#[derive(Default, Clone)]
pub struct Sources {
    files: Vec<SourceFile>,
    /// Line bounds per file, filled on demand by `index_lines`.
    index: std::cell::RefCell<std::collections::HashMap<u32, Vec<(usize, usize)>>>,
}

#[derive(Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub text: String,
}

impl Sources {
    pub fn new() -> Sources {
        Sources { files: Vec::new(), index: Default::default() }
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

    /// A file's path as a diagnostic spells it: always with `/`.
    ///
    /// Windows renders a `PathBuf` with backslashes, and the self-hosted
    /// compiler — which joins paths as strings — renders forward slashes.
    /// Two compilers that must agree byte for byte cannot disagree about a
    /// separator, and a snapshot cannot be right on one platform only.
    pub fn path(&self, id: u32) -> String {
        self.get(id).map(|f| shown(&f.path)).unwrap_or_else(|| "<unknown>".into())
    }

    /// One line of a file, walking to it from the front.
    ///
    /// Fine for a diagnostic, which happens a handful of times. The C backend
    /// asks once per place the program can fail, which on a large file made
    /// this quadratic — so it asks through [`locate`] on a `Sources` whose
    /// caller has cut the file into lines once. See `line_index`.
    fn line_text(&self, id: u32, line: u32) -> Option<&str> {
        let n = line.saturating_sub(1) as usize;
        if let Some(index) = self.index.borrow().get(&id) {
            let (a, b) = *index.get(n)?;
            // The borrow ends with this block; the slice comes from `text`,
            // which the index does not own.
            return Some(&self.get(id)?.text[a..b]);
        }
        self.get(id)?.text.lines().nth(n)
    }

    /// Cuts one file into line bounds and keeps them, so a caller asking for
    /// many lines of the same file pays for the walk once.
    pub fn index_lines(&self, id: u32) {
        if self.index.borrow().contains_key(&id) {
            return;
        }
        let Some(f) = self.get(id) else { return };
        let base = f.text.as_ptr() as usize;
        let bounds: Vec<(usize, usize)> = f
            .text
            .lines()
            .map(|l| {
                let a = l.as_ptr() as usize - base;
                (a, a + l.len())
            })
            .collect();
        self.index.borrow_mut().insert(id, bounds);
    }

    /// Where something is, and what it looks like there: the `-->` line and
    /// the quoted source under a caret.
    ///
    /// Split out from [`render`] because a compiled program says this too. A
    /// native binary cannot consult the source at run time, so the backend
    /// asks for these lines while it still can and embeds the answer — which
    /// means the failure a compiled program prints and the failure the
    /// interpreters print come from **one** implementation rather than from
    /// two that agree today. A caret placed by counting Unicode scalars, and
    /// a tab worth four columns, are exactly the rules that would drift.
    pub fn locate(&self, span: Span) -> String {
        let mut out = format!("  --> {}:{}:{}\n", self.path(span.file), span.line, span.col);
        if let Some(line) = self.line_text(span.file, span.line) {
            let num = span.line.to_string();
            let pad = " ".repeat(num.len());
            out.push_str(&format!("{} |\n", pad));
            out.push_str(&format!("{} | {}\n", num, line.replace('\t', "    ")));
            // Account for tabs expanded to 4 spaces when placing the caret.
            let prefix: String = line
                .chars()
                .take(span.col.saturating_sub(1) as usize)
                .map(|c| if c == '\t' { "    ".to_string() } else { " ".to_string() })
                .collect();
            out.push_str(&format!("{} | {}^\n", pad, prefix));
        }
        out
    }

    /// Renders a diagnostic the way rustc does: header, quoted line, caret.
    pub fn render(&self, kind: &str, d: &Diag) -> String {
        let mut out = String::new();
        out.push_str(&format!("{}: {}\n", kind, d.msg));
        out.push_str(&self.locate(d.span));
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
