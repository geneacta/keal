//! Module loading: reads a file, resolves its `import`s relative to it, and
//! splices everything into one program with a single flat namespace.
//!
//! A file is loaded at most once, so diamond imports and cycles both work.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{
    Arg, Expr, ExprKind, ImportEdge, Item, Param, Program, Stmt, StmtKind, TypeExpr,
    TypeExprKind,
};
use crate::lexer;
use crate::parser;
use crate::span::{shown, Diag, Sources, Span};

/// Traits the operators are wired to. Compiled into the binary so that a
/// program never has to import them, and written in Keal so that they are
/// nothing a user could not have declared.
const PRELUDE: &str = include_str!("prelude.keal");

thread_local! {
    /// What the editor is holding for files it has open but has not saved.
    ///
    /// The language server fills this before it loads anything, so a
    /// diagnostic is about the buffer a person is looking at rather than the
    /// file on disk. Every other command leaves it empty and reads the disk,
    /// which is what keeps the dump commands pure functions of the files.
    static OVERLAY: std::cell::RefCell<HashMap<PathBuf, String>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Hands the loader the unsaved text of the files the editor has open.
pub fn set_overlay(files: HashMap<PathBuf, String>) {
    let keyed = files.into_iter().map(|(p, t)| (overlay_key(&p), t)).collect();
    OVERLAY.with(|o| *o.borrow_mut() = keyed);
}

/// What the filesystem calls a file, rather than what somebody spelled.
///
/// Two spellings can name one file. macOS and Windows both open `lib.keal`
/// when the file on disk is `Lib.keal`, while `PathBuf` compares those as
/// different — so an editor holding `Lib.keal` and an `import "./lib.keal"`
/// would miss each other in the map, and the checker would answer from the
/// copy on disk without ever saying it had. Diagnostics one save behind,
/// with nothing to show for it.
///
/// Asking the filesystem is the only test that agrees with the filesystem,
/// on every platform and without guessing which of them fold case. A file
/// nobody has written yet has no answer to give, and there the path as
/// written is the best key there is — and the only one, so both sides still
/// agree.
fn overlay_key(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn read_source(path: &Path) -> std::io::Result<String> {
    let key = overlay_key(path);
    if let Some(text) = OVERLAY.with(|o| o.borrow().get(&key).cloned()) {
        return Ok(text);
    }
    std::fs::read_to_string(path)
}

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
    let entry_file =
        load_file(&path, None, sources, &mut seen, &mut items, &mut imports, generate)?;
    call_main(&mut items, entry_file)?;
    Ok(Program { items, imports })
}

/// Appends the call to `main`, if the entry file declared one.
///
/// A program is its top-level statements, and that is still true: this adds
/// one more, as if the last line of the entry file had written the call. It
/// is done here rather than in an engine so that the three engines inherit
/// the same program — the tree-walker, the bytecode VM and the C backend all
/// receive a call that is already in the item list, and none of them has to
/// know the name `main` at all.
///
/// Only the entry file's `main` runs. A module that declares one is a
/// library that can also be run on its own, which is a useful thing to be
/// and not a reason to run two programs at once.
fn call_main(items: &mut Vec<Item>, entry: u32) -> Result<(), Diag> {
    let Some(decl) = items.iter().find_map(|it| match it {
        Item::Fun(f) if f.name == "main" && f.span.file == entry => Some(f),
        _ => None,
    }) else {
        return Ok(());
    };

    // What a `main` may look like. Anything else is a mistake worth a
    // message: a `main` that does not match used to sit there and never
    // run, which is the silence this exists to end.
    let takes_args = match decl.params.len() {
        0 => false,
        1 if is_string_list(&decl.params[0]) => true,
        _ => {
            return Err(Diag::new(decl.span, "`main` takes no parameters, or one `List<String>`")
                .with_note(
                    "the arguments are also reachable from anywhere with `args()`",
                ))
        }
    };
    match &decl.ret {
        // `proc main()` — the program ends when it returns.
        None => {}
        // `func main(): Int` — and the Int is the exit code, as in C.
        Some(t) if names(t, "Int") => {}
        Some(_) => {
            return Err(Diag::new(
                decl.span,
                "`func main` must return `Int`, which becomes the exit code",
            )
            .with_note("declare it `proc main` if it returns nothing"))
        }
    }

    let span = decl.span;
    let returns_code = decl.ret.is_some();
    let args = if takes_args { vec![arg(call("args", vec![], span))] } else { vec![] };
    let mut expr = call("main", args, span);
    if returns_code {
        expr = call("exit", vec![arg(expr)], span);
    }
    items.push(Item::Stmt(Stmt { kind: StmtKind::Expr(expr), span }));
    Ok(())
}

fn is_string_list(p: &Param) -> bool {
    match &p.ty {
        Some(t) => match &t.kind {
            TypeExprKind::Named { name, args } => {
                name == "List" && args.len() == 1 && names(&args[0], "String")
            }
            _ => false,
        },
        None => false,
    }
}

fn names(t: &TypeExpr, want: &str) -> bool {
    matches!(&t.kind, TypeExprKind::Named { name, args } if name == want && args.is_empty())
}

fn call(name: &str, args: Vec<Arg>, span: Span) -> Expr {
    let callee = Expr {
        kind: ExprKind::Ident(name.to_string()),
        span,
        ty: None,
        inst: None,
    };
    Expr {
        kind: ExprKind::Call { callee: Box::new(callee), args },
        span,
        ty: None,
        inst: None,
    }
}

fn arg(value: Expr) -> Arg {
    Arg { name: None, value }
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

    let text = match read_source(path) {
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
/// the `.keal/deps/` of the OUTERMOST `keal.toml` above the importing file
/// — the project's, not a dependency's own, so a library reaches the same
/// copy of its dependency that everything else does. Anything else is a
/// path relative to the file that wrote it.
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
    let Some(manifest) = crate::manifest::root_of(importer) else {
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
