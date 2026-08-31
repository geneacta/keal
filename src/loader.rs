//! Module loading: reads a file, resolves its `import`s relative to it, and
//! splices everything into one program with a single flat namespace.
//!
//! A file is loaded at most once, so diamond imports and cycles both work.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{ImportEdge, Item, Program};
use crate::lexer;
use crate::parser;
use crate::span::{shown, Diag, Sources, Span};

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
    let mut seen = HashMap::new();
    let mut items = prelude(sources)?;
    let mut imports = Vec::new();
    let path = normalise(Path::new(entry));
    load_file(&path, None, sources, &mut seen, &mut items, &mut imports, generate)?;
    Ok(Program { items, imports })
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
    seen: &mut HashMap<PathBuf, u32>,
    items: &mut Vec<Item>,
    imports: &mut Vec<ImportEdge>,
    generate: bool,
) -> Result<u32, Diag> {
    // A file is read once, but an edge is recorded every time: two files
    // importing the same module are two importers.
    if let Some(id) = seen.get(path) {
        return Ok(*id);
    }

    if generate && !path.exists() && path.parent().map(|d| d.ends_with(".jbind")).unwrap_or(false)
    {
        if let Err(reason) = crate::jbind::ensure_cache(path) {
            let msg = format!("cannot generate `{}`: {}", shown(path), reason);
            return Err(match imported_from {
                Some(span) => Diag::new(span, msg),
                None => Diag::new(Span::default(), msg),
            });
        }
    }

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("cannot read `{}`: {}", shown(path), unreadable(&e));
            let msg = if path.components().any(|c| c.as_os_str() == "deps")
                && path.to_string_lossy().contains(".keal")
            {
                format!("{} -- a `dep:` import reads what is on disk: run `keal fetch` to put this project's dependencies in place", msg)
            } else if path.parent().map(|d| d.ends_with(".jbind")).unwrap_or(false) {
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
    seen.insert(path.to_path_buf(), file);
    let tokens = lexer::lex(&text, file)?;
    let program = parser::parse(tokens)?;

    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut own = Vec::new();
    for item in program.items {
        match item {
            Item::Import { path: rel, alias, span } => {
                let target = match resolve_import(&rel, &dir, path) {
                    Ok(t) => t,
                    Err(msg) => return Err(Diag::new(span, msg).with_note(
                        "run `keal fetch` to put this project's dependencies in place",
                    )),
                };
                let to =
                    load_file(&target, Some(span), sources, seen, items, imports, generate)?;
                imports.push(ImportEdge { from: file, to, alias, span });
            }
            other => own.push(other),
        }
    }
    items.extend(own);
    Ok(file)
}

/// Why a file could not be read, in the compiler's own words.
///
/// `std::io::Error`'s own text is the operating system's, in the operating
/// system's language: a French Windows says "Le fichier spécifié est
/// introuvable. (os error 2)" where macOS says "No such file or directory".
/// A diagnostic that two compilers compare byte for byte cannot be written
/// by whichever machine and locale happened to run it, so the compiler says
/// this itself and says the same thing everywhere.
fn unreadable(e: &std::io::Error) -> &'static str {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => "no such file or directory",
        PermissionDenied => "permission denied",
        IsADirectory => "that is a directory, not a file",
        _ => "it could not be read",
    }
}

/// Where an import points. `dep:name/file.keal` is a dependency, read from
/// `.keal/deps/` beside the nearest `keal.toml`; anything else is a path
/// relative to the file that wrote it.
///
/// Nothing here fetches: what is on disk is what is read, so a project that
/// commits its `.keal/deps/` builds with no network and no git at all.
fn resolve_import(rel: &str, dir: &Path, importer: &Path) -> Result<PathBuf, String> {
    let Some(rest) = rel.strip_prefix("dep:") else {
        return Ok(normalise(&dir.join(rel)));
    };
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return Err("`dep:` needs a dependency and a file, as `dep:name/file.keal`".to_string());
    }
    let Some(manifest) = crate::manifest::find(importer) else {
        return Err(format!(
            "cannot read `{}`: no `keal.toml` above `{}`, so there is no project to depend for",
            rel,
            shown(importer)
        ));
    };
    let root = manifest.parent().unwrap_or(Path::new("."));
    Ok(normalise(&root.join(".keal/deps").join(rest)))
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
