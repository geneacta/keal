//! `keal lsp` — the language server.
//!
//! An editor plugin that only colours text is a syntax file. This is the
//! other half: the thing that knows what a name means. It speaks the
//! Language Server Protocol over stdin and stdout, so one implementation
//! serves VS Code, JetBrains, Neovim, Zed and anything else that speaks it —
//! which is the whole reason to write a server rather than four plugins.
//!
//! **What it answers today.** Diagnostics as you type, the type of the thing
//! under the cursor, where a name is declared, every place it is used,
//! renaming one, the outline of a file, and completion of the names in
//! scope. What it does not do yet is stated in `docs/language.md` rather
//! than left to be discovered.
//!
//! **How it knows.** It reuses the compiler, unchanged: the loader reads the
//! editor's unsaved buffer through an overlay, the checker runs, and the
//! tree it leaves behind carries a type on every expression and a span on
//! every node. So the server has no model of the language of its own to
//! drift from the real one — a wrong answer here is a wrong answer in
//! `keal check`, and the suite already holds that to three engines.
//!
//! **What it must never do is crash.** A panicking server takes the editor's
//! Keal support down with it until someone restarts it, and this binary is
//! built with `panic = "abort"`, so there is nothing to catch. Every lookup
//! here is written to answer `None` rather than to assume.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::ast::*;
use crate::json::Json;
use crate::span::{Diag, Sources, Span};
use crate::types::Type;

pub fn run() -> ExitCode {
    let mut server = Server::default();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    loop {
        match read_message(&mut input) {
            Some(text) => {
                let Some(msg) = crate::json::parse(&text) else { continue };
                if server.handle(&msg) {
                    return ExitCode::SUCCESS;
                }
            }
            // The editor closed the pipe: that is how a session ends when
            // nobody sent `shutdown` first, and it is not an error.
            None => return ExitCode::SUCCESS,
        }
    }
}

/// One `Content-Length`-framed message, or `None` at end of input.
fn read_message(input: &mut impl Read) -> Option<String> {
    let mut length: Option<usize> = None;
    loop {
        let line = read_line(input)?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            length = rest.trim().parse::<usize>().ok();
        }
    }
    let n = length?;
    let mut body = vec![0u8; n];
    input.read_exact(&mut body).ok()?;
    String::from_utf8(body).ok()
}

fn read_line(input: &mut impl Read) -> Option<String> {
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match input.read(&mut byte) {
            Ok(0) => return (!out.is_empty()).then(|| String::from_utf8_lossy(&out).into_owned()),
            Ok(_) => {
                out.push(byte[0]);
                if byte[0] == b'\n' {
                    return Some(String::from_utf8_lossy(&out).into_owned());
                }
            }
            Err(_) => return None,
        }
    }
}

fn send(msg: Json) {
    let body = msg.to_string();
    let out = std::io::stdout();
    let mut out = out.lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

#[derive(Default)]
struct Server {
    /// What the editor is holding, by path. The server answers about these
    /// rather than about the files on disk, because they are what a person
    /// is looking at.
    open: HashMap<PathBuf, String>,
}

impl Server {
    /// Handles one message. Returns true when the session is over.
    fn handle(&mut self, msg: &Json) -> bool {
        let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
            // A response to something we sent; nothing here asks questions.
            return false;
        };
        let id = msg.get("id").cloned();
        match method {
            "initialize" => {
                reply(id, capabilities());
            }
            "initialized" => {}
            "shutdown" => reply(id, Json::Null),
            "exit" => return true,
            "textDocument/didOpen" => {
                if let (Some(path), Some(text)) = (
                    msg.at("params.textDocument.uri").and_then(uri_to_path),
                    msg.at("params.textDocument.text").and_then(|t| t.as_str()),
                ) {
                    self.open.insert(path.clone(), text.to_string());
                    self.publish(&path);
                }
            }
            "textDocument/didChange" => {
                if let Some(path) = msg.at("params.textDocument.uri").and_then(uri_to_path) {
                    // Full-text sync: the capabilities ask for it, so the
                    // last change carries the whole document.
                    if let Some(text) = msg
                        .at("params.contentChanges")
                        .and_then(|c| c.as_arr())
                        .and_then(|c| c.last())
                        .and_then(|c| c.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        self.open.insert(path.clone(), text.to_string());
                        self.publish(&path);
                    }
                }
            }
            "textDocument/didSave" => {
                if let Some(path) = msg.at("params.textDocument.uri").and_then(uri_to_path) {
                    self.publish(&path);
                }
            }
            "textDocument/didClose" => {
                if let Some(path) = msg.at("params.textDocument.uri").and_then(uri_to_path) {
                    self.open.remove(&path);
                    // Clearing them is the protocol's way of saying the file
                    // is no longer the server's to talk about.
                    send(notify(
                        "textDocument/publishDiagnostics",
                        Json::obj(vec![
                            ("uri", Json::str(path_to_uri(&path))),
                            ("diagnostics", Json::Arr(Vec::new())),
                        ]),
                    ));
                }
            }
            "textDocument/hover" => reply(id, self.hover(msg)),
            "textDocument/definition" => reply(id, self.definition(msg)),
            "textDocument/references" => reply(id, self.references(msg)),
            "textDocument/rename" => reply(id, self.rename(msg)),
            "textDocument/documentSymbol" => reply(id, self.symbols(msg)),
            "textDocument/completion" => reply(id, self.completion(msg)),
            // Anything else: a request needs an answer even when the answer
            // is nothing, or the editor waits forever.
            _ => {
                if id.is_some() {
                    reply(id, Json::Null);
                }
            }
        }
        false
    }

    /// Loads and checks a file the way `keal check` would, with the editor's
    /// unsaved buffers in front of the disk.
    fn analyse(&self, path: &Path) -> Option<Analysis> {
        crate::loader::set_overlay(self.open.clone());
        let mut sources = Sources::new();
        let entry = path.to_string_lossy().into_owned();
        let mut program = match crate::loader::load(&entry, &mut sources) {
            Ok(p) => p,
            Err(d) => {
                return Some(Analysis {
                    errors: vec![d],
                    warnings: Vec::new(),
                    index: Index::default(),
                    sources,
                })
            }
        };
        let (errors, warnings) = crate::checker::check(&mut program, &sources);
        let mut index = Index::default();
        index.walk_program(&program, &sources);
        Some(Analysis { errors, warnings, index, sources })
    }

    fn publish(&self, path: &Path) {
        let Some(a) = self.analyse(path) else { return };
        // Every file the load touched gets its own list, so an error in an
        // imported module is reported on the module rather than on the
        // import that reached it.
        let mut by_file: HashMap<u32, Vec<Json>> = HashMap::new();
        for (severity, list) in [(1, &a.errors), (2, &a.warnings)] {
            for d in list {
                by_file
                    .entry(d.span.file)
                    .or_default()
                    .push(diagnostic(d, severity, &a.sources));
            }
        }
        // The file the editor asked about always gets an answer, even an
        // empty one — that is what clears the squiggles it drew before.
        let mut files: Vec<u32> = by_file.keys().copied().collect();
        let this_file = (0..a.sources.len() as u32)
            .find(|id| a.sources.get(*id).map(|f| f.path == path).unwrap_or(false));
        if let Some(id) = this_file {
            if !files.contains(&id) {
                files.push(id);
            }
        }
        for id in files {
            let Some(file) = a.sources.get(id) else { continue };
            // The prelude is compiled in; there is no file to point at.
            if file.path.as_os_str().is_empty() || file.path.to_string_lossy() == "<prelude>" {
                continue;
            }
            let items = by_file.remove(&id).unwrap_or_default();
            send(notify(
                "textDocument/publishDiagnostics",
                Json::obj(vec![
                    ("uri", Json::str(path_to_uri(&file.path))),
                    ("diagnostics", Json::Arr(items)),
                ]),
            ));
        }
    }

    /// The file, the analysis and the span the cursor is on.
    fn at_cursor(&self, msg: &Json) -> Option<(PathBuf, Analysis, u32, u32, u32)> {
        let path = msg.at("params.textDocument.uri").and_then(uri_to_path)?;
        let line = msg.at("params.position.line")?.as_i64()? as u32;
        let character = msg.at("params.position.character")?.as_i64()? as u32;
        let a = self.analyse(&path)?;
        let file = (0..a.sources.len() as u32)
            .find(|id| a.sources.get(*id).map(|f| f.path == path).unwrap_or(false))?;
        let text = &a.sources.get(file)?.text;
        let col = utf16_to_byte_col(text, line, character)?;
        Some((path, a, file, line + 1, col))
    }

    fn hover(&self, msg: &Json) -> Json {
        let Some((_, a, file, line, col)) = self.at_cursor(msg) else { return Json::Null };
        // A declaration is not an expression, so it carries no type of its
        // own — but hovering the name a program just bound is the most
        // ordinary thing to do, so that is answered first.
        let mut text = a
            .index
            .name_at(file, line, col)
            .and_then(|n| a.index.decl_of(&n).and_then(|d| d.detail.clone()));
        if text.is_none() {
            text = a.index.innermost(file, line, col).and_then(|e| e.detail.clone());
        }
        let Some(text) = text else { return Json::Null };
        Json::obj(vec![(
            "contents",
            Json::obj(vec![
                ("kind", Json::str("markdown")),
                ("value", Json::str(format!("```keal\n{}\n```", text))),
            ]),
        )])
    }

    fn definition(&self, msg: &Json) -> Json {
        let Some((_, a, file, line, col)) = self.at_cursor(msg) else { return Json::Null };
        let Some(name) = a.index.name_at(file, line, col) else { return Json::Null };
        let Some(decl) = a.index.decl_of(&name) else { return Json::Null };
        let Some(f) = a.sources.get(decl.span.file) else { return Json::Null };
        Json::obj(vec![
            ("uri", Json::str(path_to_uri(&f.path))),
            ("range", range_of(decl.span, decl.name.len(), &a.sources)),
        ])
    }

    fn references(&self, msg: &Json) -> Json {
        let Some((_, a, file, line, col)) = self.at_cursor(msg) else { return Json::Null };
        let Some(name) = a.index.name_at(file, line, col) else { return Json::Null };
        Json::Arr(self.locations_of(&a, &name))
    }

    fn locations_of(&self, a: &Analysis, name: &str) -> Vec<Json> {
        let mut out = Vec::new();
        for u in a.index.uses.iter().filter(|u| u.name == name) {
            let Some(f) = a.sources.get(u.span.file) else { continue };
            out.push(Json::obj(vec![
                ("uri", Json::str(path_to_uri(&f.path))),
                ("range", range_of(u.span, name.len(), &a.sources)),
            ]));
        }
        if let Some(d) = a.index.decl_of(name) {
            if let Some(f) = a.sources.get(d.span.file) {
                out.insert(
                    0,
                    Json::obj(vec![
                        ("uri", Json::str(path_to_uri(&f.path))),
                        ("range", range_of(d.span, name.len(), &a.sources)),
                    ]),
                );
            }
        }
        out
    }

    fn rename(&self, msg: &Json) -> Json {
        let Some((_, a, file, line, col)) = self.at_cursor(msg) else { return Json::Null };
        let Some(new) = msg.at("params.newName").and_then(|n| n.as_str()) else {
            return Json::Null;
        };
        let Some(name) = a.index.name_at(file, line, col) else { return Json::Null };
        // Every edit, grouped by the file it belongs to — which is what the
        // protocol wants and what makes the change one undo step.
        let mut by_uri: HashMap<String, Vec<Json>> = HashMap::new();
        for loc in self.locations_of(&a, &name) {
            let Some(uri) = loc.get("uri").and_then(|u| u.as_str()) else { continue };
            let Some(range) = loc.get("range") else { continue };
            by_uri.entry(uri.to_string()).or_default().push(Json::obj(vec![
                ("range", range.clone()),
                ("newText", Json::str(new)),
            ]));
        }
        let mut changes = std::collections::BTreeMap::new();
        for (uri, edits) in by_uri {
            changes.insert(uri, Json::Arr(edits));
        }
        Json::obj(vec![("changes", Json::Obj(changes))])
    }

    fn symbols(&self, msg: &Json) -> Json {
        let Some(path) = msg.at("params.textDocument.uri").and_then(uri_to_path) else {
            return Json::Null;
        };
        let Some(a) = self.analyse(&path) else { return Json::Null };
        let Some(file) = (0..a.sources.len() as u32)
            .find(|id| a.sources.get(*id).map(|f| f.path == path).unwrap_or(false))
        else {
            return Json::Null;
        };
        let mut out = Vec::new();
        for d in a.index.decls.iter().filter(|d| d.span.file == file && d.top_level) {
            out.push(Json::obj(vec![
                ("name", Json::str(&d.name)),
                ("kind", Json::int(d.symbol_kind)),
                ("range", range_of(d.span, d.name.len(), &a.sources)),
                ("selectionRange", range_of(d.span, d.name.len(), &a.sources)),
            ]));
        }
        Json::Arr(out)
    }

    fn completion(&self, msg: &Json) -> Json {
        let Some(path) = msg.at("params.textDocument.uri").and_then(uri_to_path) else {
            return Json::Null;
        };
        let Some(a) = self.analyse(&path) else { return Json::Null };
        let mut out = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for d in &a.index.decls {
            if seen.contains(&d.name.as_str()) || d.name.contains('#') {
                continue;
            }
            seen.push(&d.name);
            let mut pairs = vec![
                ("label", Json::str(&d.name)),
                ("kind", Json::int(d.completion_kind)),
            ];
            if let Some(detail) = &d.detail {
                pairs.push(("detail", Json::str(detail)));
            }
            out.push(Json::obj(pairs));
        }
        for word in KEYWORDS {
            out.push(Json::obj(vec![
                ("label", Json::str(*word)),
                ("kind", Json::int(14)),
            ]));
        }
        Json::Arr(out)
    }
}

/// The words a completion offers beside the names in scope. Kept here rather
/// than read from the lexer because the lexer's list includes the ones held
/// for later, and offering a word that is refused where it is written would
/// be worse than offering nothing.
const KEYWORDS: &[&str] = &[
    "val", "var", "func", "proc", "class", "record", "trait", "enum", "macro", "constexpr",
    "import", "public", "package", "private", "if", "unless", "else", "when", "while", "for",
    "in", "break", "continue", "return", "try", "catch", "throw", "true", "false", "null",
    "this", "is", "not", "and", "or", "xor", "xnor", "nand", "nor", "implies", "weak", "extern",
    "native", "deinit",
];

struct Analysis {
    errors: Vec<Diag>,
    warnings: Vec<Diag>,
    index: Index,
    sources: Sources,
}

/// Where a name is declared, and how it should be described.
struct Decl {
    name: String,
    span: Span,
    detail: Option<String>,
    top_level: bool,
    /// The protocol's `SymbolKind`, and its `CompletionItemKind` — two
    /// different numberings for the same idea, so both are recorded rather
    /// than guessed at the call site.
    symbol_kind: i64,
    completion_kind: i64,
}

struct Entry {
    span: Span,
    detail: Option<String>,
}

struct Use {
    name: String,
    span: Span,
}

/// What the server knows about a program after the checker has run: where
/// each name is declared, everywhere it is used, and the type of every
/// expression that has one.
#[derive(Default)]
struct Index {
    decls: Vec<Decl>,
    uses: Vec<Use>,
    typed: Vec<Entry>,
    /// Every file's text, so a declaration's span can be moved from the
    /// keyword to the name. `val here = ...` spans the `val`, and hovering
    /// the `val` is not what anybody does.
    lines: Vec<Vec<String>>,
}

impl Index {
    fn decl_of(&self, name: &str) -> Option<&Decl> {
        self.decls.iter().find(|d| d.name == name)
    }

    /// The name written at a position, whether it is a use or the
    /// declaration itself — so go-to-definition on a declaration is a
    /// no-op rather than nothing.
    fn name_at(&self, file: u32, line: u32, col: u32) -> Option<String> {
        let mut best: Option<(&str, u32)> = None;
        for u in &self.uses {
            if u.span.file == file && u.span.line == line && u.span.col <= col {
                let end = u.span.col + u.name.chars().count() as u32;
                if col <= end && best.map(|(_, c)| u.span.col >= c).unwrap_or(true) {
                    best = Some((&u.name, u.span.col));
                }
            }
        }
        for d in &self.decls {
            if d.span.file == file && d.span.line == line && d.span.col <= col {
                let end = d.span.col + d.name.chars().count() as u32;
                if col <= end && best.map(|(_, c)| d.span.col >= c).unwrap_or(true) {
                    best = Some((&d.name, d.span.col));
                }
            }
        }
        best.map(|(n, _)| n.to_string())
    }

    /// The narrowest thing recorded at a position. Spans here are points
    /// rather than ranges, so "narrowest" means the one that starts latest
    /// at or before the cursor — which is the innermost expression.
    fn innermost(&self, file: u32, line: u32, col: u32) -> Option<&Entry> {
        let mut best: Option<&Entry> = None;
        for e in &self.typed {
            if e.span.file != file || e.span.line != line || e.span.col > col {
                continue;
            }
            if best.map(|b| e.span.col >= b.span.col).unwrap_or(true) {
                best = Some(e);
            }
        }
        best
    }

    fn declare(
        &mut self,
        name: &str,
        span: Span,
        detail: Option<String>,
        top_level: bool,
        symbol_kind: i64,
        completion_kind: i64,
    ) {
        let span = self.at_name(span, name);
        self.decls.push(Decl {
            name: name.to_string(),
            span,
            detail,
            top_level,
            symbol_kind,
            completion_kind,
        });
    }

    /// A declaration's span points at the word that opened it — `val`,
    /// `func`, `enum`. The name is what a person points at, so it is found
    /// on the same line, at or after that column.
    fn at_name(&self, span: Span, name: &str) -> Span {
        let Some(file) = self.lines.get(span.file as usize) else { return span };
        let Some(line) = file.get(span.line.saturating_sub(1) as usize) else { return span };
        let from = span.col.saturating_sub(1) as usize;
        if from > line.len() {
            return span;
        }
        match line[from..].find(name) {
            Some(off) => Span::new(span.file, span.line, (from + off) as u32 + 1),
            None => span,
        }
    }

    fn walk_program(&mut self, program: &Program, sources: &Sources) {
        self.lines = (0..sources.len() as u32)
            .map(|id| {
                sources
                    .get(id)
                    .map(|f| f.text.lines().map(|l| l.to_string()).collect())
                    .unwrap_or_default()
            })
            .collect();
        for item in &program.items {
            match item {
                Item::Fun(f) => {
                    self.declare(&f.name, f.span, Some(signature(f)), true, 12, 3);
                    self.walk_fun(f);
                }
                Item::Class(c) => {
                    let kind = if c.is_record { "record" } else { "class" };
                    self.declare(
                        &c.name,
                        c.span,
                        Some(format!("{} {}", kind, c.name)),
                        true,
                        5,
                        7,
                    );
                    for p in &c.ctor {
                        self.declare(&p.name, p.span, None, false, 8, 5);
                    }
                    for f in &c.fields {
                        self.declare(&f.name, f.span, None, false, 8, 5);
                        if let Some(init) = &f.init {
                            self.walk_expr(init);
                        }
                    }
                    for m in &c.methods {
                        self.declare(&m.name, m.span, Some(signature(m)), false, 6, 2);
                        self.walk_fun(m);
                    }
                }
                Item::Trait(t) => {
                    self.declare(&t.name, t.span, Some(format!("trait {}", t.name)), true, 11, 8);
                }
                Item::Enum(en) => {
                    let names: Vec<&str> = en.variants.iter().map(|v| v.name.as_str()).collect();
                    self.declare(
                        &en.name,
                        en.span,
                        Some(format!("enum {} {{ {} }}", en.name, names.join(", "))),
                        true,
                        10,
                        13,
                    );
                    for v in &en.variants {
                        self.declare(
                            &v.name,
                            v.span,
                            Some(format!("{}.{}", en.name, v.name)),
                            false,
                            22,
                            20,
                        );
                    }
                }
                Item::Macro(m) => {
                    let ps = m.params.join(", ");
                    self.declare(
                        &m.name,
                        m.span,
                        Some(format!("macro {}({})", m.name, ps)),
                        true,
                        12,
                        3,
                    );
                    self.walk_block(&m.body);
                }
                Item::Extern(x) => {
                    self.declare(&x.name, x.span, Some(format!("extern func {}", x.name)), true, 12, 3);
                }
                Item::Stmt(s) => self.walk_stmt(s, true),
                Item::Native { .. } | Item::Import { .. } => {}
            }
        }
    }

    fn walk_fun(&mut self, f: &FunDecl) {
        for p in f.params.iter() {
            self.declare(&p.name, p.span, None, false, 13, 6);
            if let Some(d) = &p.default {
                self.walk_expr(d);
            }
        }
        self.walk_block(&f.body);
    }

    fn walk_block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.walk_stmt(s, false);
        }
    }

    fn walk_stmt(&mut self, s: &Stmt, top: bool) {
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                let detail = init.ty.as_ref().map(|t| format!("{}: {}", name, t));
                self.declare(name, s.span, detail, top, 13, 6);
                self.walk_expr(init);
            }
            StmtKind::Destructure { pattern, init, .. } => {
                for b in pattern.binds.iter().flatten() {
                    self.declare(b, pattern.span, None, top, 13, 6);
                }
                self.walk_expr(init);
            }
            StmtKind::Block(b) => self.walk_block(b),
            StmtKind::Expr(e) | StmtKind::Throw(e) => self.walk_expr(e),
            StmtKind::Return(Some(e)) => self.walk_expr(e),
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
            StmtKind::While { cond, body } => {
                self.walk_expr(cond);
                self.walk_block(body);
            }
            StmtKind::For { var, iter, body, .. } => {
                self.declare(var, s.span, None, false, 13, 6);
                self.walk_expr(iter);
                self.walk_block(body);
            }
            StmtKind::Try { body, clauses } => {
                self.walk_block(body);
                for c in clauses {
                    self.declare(&c.name, c.span, None, false, 13, 6);
                    self.walk_block(&c.handler);
                }
            }
            StmtKind::Fun(f) => {
                self.declare(&f.name, f.span, Some(signature(f)), false, 12, 3);
                self.walk_fun(f);
            }
            StmtKind::Class(_) => {}
        }
    }

    fn walk_expr(&mut self, e: &Expr) {
        if let Some(t) = &e.ty {
            self.typed.push(Entry { span: e.span, detail: Some(describe(e, t)) });
        }
        match &e.kind {
            ExprKind::Ident(name) => {
                self.uses.push(Use { name: name.clone(), span: e.span });
            }
            // The enum's own name is written just before the dot, and
            // pointing at it should reach the enum. Nothing records that
            // span, so it is computed: the variant's span is the name after
            // the dot, and the enum ends one character before it.
            ExprKind::Variant { enm, .. } => {
                let start = e.span.col.saturating_sub(1 + enm.chars().count() as u32);
                if start >= 1 {
                    self.uses.push(Use {
                        name: enm.to_string(),
                        span: Span::new(e.span.file, e.span.line, start),
                    });
                }
            }
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Null
            | ExprKind::This => {}
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.walk_expr(x);
                    }
                }
            }
            ExprKind::Unary { rhs, .. } => self.walk_expr(rhs),
            ExprKind::NotNull(inner) => self.walk_expr(inner),
            ExprKind::Binary { lhs, rhs, .. }
            | ExprKind::Logical { lhs, rhs, .. }
            | ExprKind::Elvis { lhs, rhs } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            ExprKind::Assign { target, value, .. } => {
                self.walk_expr(target);
                self.walk_expr(value);
            }
            ExprKind::Call { callee, args } => {
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(&a.value);
                }
            }
            ExprKind::MacroCall { args, .. } => {
                for a in args {
                    self.walk_expr(a);
                }
            }
            ExprKind::Field { obj, .. } => self.walk_expr(obj),
            ExprKind::MethodCall { obj, args, .. } => {
                self.walk_expr(obj);
                for a in args {
                    self.walk_expr(&a.value);
                }
            }
            ExprKind::Index { obj, index } => {
                self.walk_expr(obj);
                self.walk_expr(index);
            }
            ExprKind::If { cond, then, els } => {
                self.walk_expr(cond);
                self.walk_block(then);
                match els.as_deref() {
                    Some(Else::Block(b)) => self.walk_block(b),
                    Some(Else::If(x)) => self.walk_expr(x),
                    None => {}
                }
            }
            ExprKind::Ternary { cond, branches } => {
                self.walk_expr(cond);
                for b in branches {
                    self.walk_expr(b);
                }
            }
            ExprKind::When { subject, arms } => {
                if let Some(s) = subject {
                    self.walk_expr(s);
                }
                for arm in arms {
                    if let WhenPattern::Values(vs) = &arm.pattern {
                        for v in vs {
                            self.walk_expr(v);
                        }
                    }
                    if let Some(g) = &arm.guard {
                        self.walk_expr(g);
                    }
                    self.walk_block(&arm.body);
                }
            }
            ExprKind::ListLit(items) => {
                for i in items {
                    self.walk_expr(i);
                }
            }
            ExprKind::MapLit(entries) => {
                for (k, v) in entries {
                    self.walk_expr(k);
                    self.walk_expr(v);
                }
            }
            ExprKind::Lambda { params, body } => {
                for p in params.iter() {
                    self.declare(&p.name, p.span, None, false, 13, 6);
                }
                self.walk_block(body);
            }
            ExprKind::Range { start, end } => {
                self.walk_expr(start);
                self.walk_expr(end);
            }
            ExprKind::Is { value, .. } => self.walk_expr(value),
        }
    }
}

/// What hover shows: the type, and the name it belongs to where there is
/// one, because `x: Int` reads better than `Int` alone.
fn describe(e: &Expr, t: &Type) -> String {
    match &e.kind {
        ExprKind::Ident(name) => format!("{}: {}", name, t),
        ExprKind::Field { name, .. } => format!(".{}: {}", name, t),
        ExprKind::MethodCall { name, .. } => format!(".{}(...): {}", name, t),
        ExprKind::Variant { enm, name, .. } => format!("{}.{}: {}", enm, name, t),
        _ => t.to_string(),
    }
}

fn signature(f: &FunDecl) -> String {
    let word = if f.ret.is_some() { "func" } else { "proc" };
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| match &p.ty {
            Some(t) => format!("{}{}: {}", if p.mutable { "var " } else { "" }, p.name, type_text(t)),
            None => p.name.clone(),
        })
        .collect();
    match &f.ret {
        Some(r) => format!("{} {}({}): {}", word, f.name, params.join(", "), type_text(r)),
        None => format!("{} {}({})", word, f.name, params.join(", ")),
    }
}

/// A written type as the program spelled it. The checker's resolved `Type`
/// is the better answer where there is one, but a signature is read before
/// anything is checked.
fn type_text(t: &TypeExpr) -> String {
    match &t.kind {
        TypeExprKind::Named { name, args } if args.is_empty() => name.clone(),
        TypeExprKind::Named { name, args } => {
            let parts: Vec<String> = args.iter().map(type_text).collect();
            format!("{}<{}>", name, parts.join(", "))
        }
        TypeExprKind::Nullable(inner) => format!("{}?", type_text(inner)),
        TypeExprKind::Fun { params, ret } => {
            let parts: Vec<String> = params.iter().map(type_text).collect();
            format!("({}) -> {}", parts.join(", "), type_text(ret))
        }
        TypeExprKind::Boundary { mode, inner } => format!("{} {}", mode, type_text(inner)),
    }
}

fn capabilities() -> Json {
    Json::obj(vec![(
        "capabilities",
        Json::obj(vec![
            // Full sync: a Keal file is small enough that sending the whole
            // buffer costs less than tracking incremental edits correctly.
            ("textDocumentSync", Json::int(1)),
            ("hoverProvider", Json::Bool(true)),
            ("definitionProvider", Json::Bool(true)),
            ("referencesProvider", Json::Bool(true)),
            ("renameProvider", Json::Bool(true)),
            ("documentSymbolProvider", Json::Bool(true)),
            (
                "completionProvider",
                Json::obj(vec![("triggerCharacters", Json::Arr(vec![Json::str(".")]))]),
            ),
        ]),
    )])
}

fn reply(id: Option<Json>, result: Json) {
    let Some(id) = id else { return };
    send(Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("id", id),
        ("result", result),
    ]));
}

fn notify(method: &str, params: Json) -> Json {
    Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("method", Json::str(method)),
        ("params", params),
    ])
}

fn diagnostic(d: &Diag, severity: i64, sources: &Sources) -> Json {
    let mut message = d.msg.clone();
    if let Some(note) = &d.note {
        // The note is where a Keal diagnostic says what to do about it, and
        // an editor that dropped it would be showing half the sentence.
        message.push_str("\n\nnote: ");
        message.push_str(note);
    }
    Json::obj(vec![
        ("range", range_of(d.span, word_len(d.span, sources), sources)),
        ("severity", Json::int(severity)),
        ("source", Json::str("keal")),
        ("message", Json::str(message)),
    ])
}

/// How wide to draw a squiggle. A Keal span is a point, so the width comes
/// from the source: the identifier or number that starts there, or one
/// character when it starts on something else.
fn word_len(span: Span, sources: &Sources) -> usize {
    let Some(file) = sources.get(span.file) else { return 1 };
    let Some(line) = file.text.lines().nth(span.line.saturating_sub(1) as usize) else {
        return 1;
    };
    let start = span.col.saturating_sub(1) as usize;
    if start >= line.len() {
        return 1;
    }
    let rest = &line[start..];
    let n = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .count();
    n.max(1)
}

/// A Keal span, as the protocol wants it: zero-based lines, and characters
/// counted in UTF-16 code units — which is not the same as bytes the moment
/// a program has an accent in it.
fn range_of(span: Span, len: usize, sources: &Sources) -> Json {
    let line = span.line.saturating_sub(1);
    let start = byte_to_utf16_col(sources, span.file, line, span.col.saturating_sub(1) as usize);
    Json::obj(vec![
        (
            "start",
            Json::obj(vec![("line", Json::int(line as i64)), ("character", Json::int(start as i64))]),
        ),
        (
            "end",
            Json::obj(vec![
                ("line", Json::int(line as i64)),
                ("character", Json::int((start + len) as i64)),
            ]),
        ),
    ])
}

fn byte_to_utf16_col(sources: &Sources, file: u32, line: u32, byte_col: usize) -> usize {
    let Some(f) = sources.get(file) else { return byte_col };
    let Some(text) = f.text.lines().nth(line as usize) else { return byte_col };
    let upto = text.get(..byte_col.min(text.len())).unwrap_or(text);
    upto.chars().map(|c| c.len_utf16()).sum()
}

fn utf16_to_byte_col(text: &str, line: u32, character: u32) -> Option<u32> {
    let line_text = text.lines().nth(line as usize)?;
    let mut units = 0u32;
    for (i, c) in line_text.char_indices() {
        if units >= character {
            return Some(i as u32 + 1);
        }
        units += c.len_utf16() as u32;
    }
    Some(line_text.len() as u32 + 1)
}

/// `file:///path/to/x.keal` — with the percent-escapes an editor uses for
/// spaces and anything else outside the unreserved set.
fn uri_to_path(uri: &Json) -> Option<PathBuf> {
    let uri = uri.as_str()?;
    let rest = uri.strip_prefix("file://")?;
    // `file://host/path` is not something an editor sends for a local file,
    // so anything before the first `/` is an empty authority.
    let rest = match rest.find('/') {
        Some(i) => &rest[i..],
        None => rest,
    };
    let mut out = String::new();
    let bytes: Vec<char> = rest.chars().collect();
    let mut i = 0;
    let mut raw: Vec<u8> = Vec::new();
    while i < bytes.len() {
        if bytes[i] == '%' && i + 2 < bytes.len() {
            let hi = bytes[i + 1].to_digit(16);
            let lo = bytes[i + 2].to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                raw.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        let mut buf = [0u8; 4];
        raw.extend_from_slice(bytes[i].encode_utf8(&mut buf).as_bytes());
        i += 1;
    }
    out.push_str(&String::from_utf8_lossy(&raw));
    // Windows sends `/C:/x`, and a leading slash there is not part of the
    // path. It also sends `/` throughout, where the rest of the process
    // deals in `\` — and this path is a `HashMap` key, so the two spellings
    // have to become one here rather than be trusted to compare equal.
    let out = if cfg!(windows) {
        let out = if out.len() > 2 && out.as_bytes()[0] == b'/' && out.as_bytes()[2] == b':' {
            out[1..].to_string()
        } else {
            out
        };
        out.replace('/', "\\")
    } else {
        out
    };
    Some(PathBuf::from(out))
}

fn path_to_uri(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf())
    };
    let text = absolute.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for c in text.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | '/' | ':' => out.push(c),
            other => {
                let mut buf = [0u8; 4];
                for b in other.encode_utf8(&mut buf).as_bytes() {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("%{:02X}", b));
                }
            }
        }
    }
    out
}
