//! Module loading: reads a file, resolves its `import`s relative to it, and
//! splices everything into one program with a single flat namespace.
//!
//! A file is loaded at most once, so diamond imports and cycles both work.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ast::{Item, Program};
use crate::lexer;
use crate::parser;
use crate::span::{Diag, Sources, Span};

pub fn load(entry: &str, sources: &mut Sources) -> Result<Program, Diag> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    let path = normalise(Path::new(entry));
    load_file(&path, None, sources, &mut seen, &mut items)?;
    Ok(Program { items })
}

/// Loads `path` and everything it imports, appending to `items` so that a
/// module's declarations always precede the file that imported it.
fn load_file(
    path: &Path,
    imported_from: Option<Span>,
    sources: &mut Sources,
    seen: &mut HashSet<PathBuf>,
    items: &mut Vec<Item>,
) -> Result<(), Diag> {
    if !seen.insert(path.to_path_buf()) {
        return Ok(());
    }

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("cannot read `{}`: {}", path.display(), e);
            return Err(match imported_from {
                Some(span) => Diag::new(span, msg),
                // No import site: the failure is the entry point itself.
                None => Diag::new(Span::default(), msg),
            });
        }
    };

    let file = sources.add(path, text.clone());
    let tokens = lexer::lex(&text, file)?;
    let program = parser::parse(tokens)?;

    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut own = Vec::new();
    for item in program.items {
        match item {
            Item::Import { path: rel, span } => {
                let target = normalise(&dir.join(&rel));
                load_file(&target, Some(span), sources, seen, items)?;
            }
            other => own.push(other),
        }
    }
    items.extend(own);
    Ok(())
}

/// Collapses `.` and `..` segments so the same file is never loaded twice
/// under two different spellings. Unlike `canonicalize` this does not need
/// the file to exist, which keeps the "cannot read" error message useful.
fn normalise(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in p.components() {
        use std::path::Component::*;
        match part {
            CurDir => {}
            ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}
