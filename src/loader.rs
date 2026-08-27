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

/// Traits the operators are wired to. Compiled into the binary so that a
/// program never has to import them, and written in Keal so that they are
/// nothing a user could not have declared.
const PRELUDE: &str = include_str!("prelude.keal");

pub fn load(entry: &str, sources: &mut Sources) -> Result<Program, Diag> {
    load_inner(entry, sources, false)
}

/// Like `load`, but a missing `.jbind/` module is generated on the spot
/// (through `javap`, so a JDK must be installed). Only the running commands
/// take this path — the dump commands stay pure functions of the files on
/// disk, so the self-hosting corpora compare the same inputs on both sides.
pub fn load_generating(entry: &str, sources: &mut Sources) -> Result<Program, Diag> {
    load_inner(entry, sources, true)
}

fn load_inner(entry: &str, sources: &mut Sources, generate: bool) -> Result<Program, Diag> {
    let mut seen = HashSet::new();
    let mut items = prelude(sources)?;
    let path = normalise(Path::new(entry));
    load_file(&path, None, sources, &mut seen, &mut items, generate)?;
    Ok(Program { items })
}

/// Parses the prelude and registers it with `sources`, so a diagnostic that
/// points into it still renders.
pub fn prelude(sources: &mut Sources) -> Result<Vec<Item>, Diag> {
    let file = sources.add("<prelude>", PRELUDE.to_string());
    let tokens = lexer::lex(PRELUDE, file)?;
    Ok(parser::parse(tokens)?.items)
}

/// Loads `path` and everything it imports, appending to `items` so that a
/// module's declarations always precede the file that imported it.
fn load_file(
    path: &Path,
    imported_from: Option<Span>,
    sources: &mut Sources,
    seen: &mut HashSet<PathBuf>,
    items: &mut Vec<Item>,
    generate: bool,
) -> Result<(), Diag> {
    if !seen.insert(path.to_path_buf()) {
        return Ok(());
    }

    if generate && !path.exists() && path.parent().map(|d| d.ends_with(".jbind")).unwrap_or(false)
    {
        if let Err(reason) = crate::jbind::ensure_cache(path) {
            let msg = format!("cannot generate `{}`: {}", path.display(), reason);
            return Err(match imported_from {
                Some(span) => Diag::new(span, msg),
                None => Diag::new(Span::default(), msg),
            });
        }
    }

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("cannot read `{}`: {}", path.display(), e);
            let msg = if path.parent().map(|d| d.ends_with(".jbind")).unwrap_or(false) {
                format!("{} -- `import java.time.LocalDate`-style modules are generated: run `keal jbind --cache` for this import, or run/build with a JDK installed", msg)
            } else {
                msg
            };
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
                load_file(&target, Some(span), sources, seen, items, generate)?;
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
