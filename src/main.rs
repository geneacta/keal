//! The `keal` command-line driver: module loading, running, checking, REPL.

mod ast;
mod astdump;
mod bindgen;
mod jbind;
mod doctor;
mod kealdoc;
mod builtins;
mod bytecode;
mod cbackend;
mod checker;
mod compiler;
mod interp;
mod layout;
mod lexer;
mod loader;
mod native;
mod nativebuild;
mod parser;
mod repl;
mod runtime;
mod span;
mod types;
mod value;
mod vm;

use std::process::ExitCode;

use span::Sources;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
keal — a small statically typed language

usage:
    keal <file.keal>          run a program
    keal run <file.keal>      run a program
    keal check <file.keal>    type-check without running
    keal layout <file.keal>   show how the program's values are laid out
    keal tokens <file.keal>   dump the token stream (the self-hosting oracle)
    keal ast <file.keal>      dump the parse tree (likewise)
    keal types <file.keal>    dump the checked, typed tree (likewise)
    keal cgen <file.keal>     emit C with compact refusals (likewise)
    keal emit-header <f.keal>  print a C header for the program's boundary
    keal emit-c <file.keal>   print the C a native build would compile
    keal bindgen <header.h>   turn a C header into extern declarations
    keal doc [files...]       render /// comments and signatures to one
                              self-contained HTML page (-o file.html);
                              with no files, document the standard library
    keal doctor               report the interop toolchains found on this
                              machine, next to the versions the tests
                              were last verified against
    keal jbind <java.Class>... generate typed Keal wrappers for Java
                              classes over lib/jvm.keal (needs javap;
                              --jvm <path> sets the emitted import path,
                              --cache <dir> writes the module a no-path
                              `import java.time.LocalDate` loads)
    keal build <file.keal> [sources... libs... flags...]
                              compile to a native executable; extras may be
                              C/C++ sources, .a/.so/.o link inputs, -l/-L
                              linker flags and -I/-D compile flags
    keal repl                 start an interactive session
    keal version              print the version
";

/// The evaluator recurses once per Keal call, so a deeply recursive program
/// needs far more stack than a thread gets by default. Reserving it here (it
/// is virtual until touched) lets `MAX_DEPTH` be the limit users actually hit,
/// with a clean error instead of a crash.
const STACK_SIZE: usize = 512 * 1024 * 1024;

fn main() -> ExitCode {
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(real_main)
        .expect("cannot start the interpreter thread")
        .join()
        .unwrap_or(ExitCode::FAILURE)
}

/// Which engine runs the program. The tree-walker is kept as the reference
/// implementation: the test suite runs every program through both and the
/// two must agree.
#[derive(Clone, Copy, PartialEq)]
enum Engine {
    Bytecode,
    Ast,
}

fn real_main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut engine = Engine::Bytecode;
    args.retain(|a| match a.as_str() {
        "--ast" => {
            engine = Engine::Ast;
            false
        }
        "--vm" => {
            engine = Engine::Bytecode;
            false
        }
        _ => true,
    });
    if args.first().map(|a| a.as_str()) == Some("jbind") {
        return jbind::run(&args[1..]);
    }
    if args.first().map(|a| a.as_str()) == Some("doc") {
        return kealdoc::run(&args[1..]);
    }
    if args.first().map(|a| a.as_str()) == Some("doctor") {
        return doctor::run();
    }
    let (command, target) = match args.as_slice() {
        [] => ("repl", None),
        [one] if one == "repl" => ("repl", None),
        [one] if one == "version" || one == "--version" || one == "-V" => ("version", None),
        [one] if one == "help" || one == "--help" || one == "-h" => ("help", None),
        // A bare path runs it; the named subcommands keep their meaning.
        [one, ..]
            if !one.starts_with('-')
                && !matches!(
                    one.as_str(),
                    "run" | "check" | "layout" | "emit-c" | "build" | "repl" | "version"
                        | "help" | "tokens" | "ast" | "types" | "cgen" | "emit-header"
                        | "bindgen"
                ) =>
        {
            ("run", Some(one.clone()))
        }
        [cmd, file, ..]
            if matches!(
                cmd.as_str(),
                "run" | "check" | "layout" | "emit-c" | "build" | "tokens" | "ast" | "types"
                    | "cgen" | "emit-header" | "bindgen"
            ) =>
        {
            (
                match cmd.as_str() {
                    "run" => "run",
                    "check" => "check",
                    "layout" => "layout",
                    "emit-c" => "emit-c",
                    "tokens" => "tokens",
                    "ast" => "ast",
                    "types" => "types",
                    "cgen" => "cgen",
                    "emit-header" => "emit-header",
                    "bindgen" => "bindgen",
                    _ => "build",
                },
                Some(file.clone()),
            )
        }
        _ => ("help", None),
    };

    match command {
        "version" => {
            println!("keal {}", VERSION);
            ExitCode::SUCCESS
        }
        "help" => {
            print!("{}", USAGE);
            ExitCode::SUCCESS
        }
        "repl" => repl::run(),
        "layout" => show_layout(&target.unwrap()),
        "tokens" => dump_tokens(&target.unwrap()),
        "ast" => dump_ast(&target.unwrap()),
        "types" => dump_types(&target.unwrap()),
        "cgen" => dump_cgen(&target.unwrap()),
        "emit-header" => emit_header(&target.unwrap()),
        "bindgen" => bindgen::run(&target.unwrap()),
        "emit-c" => nativebuild::emit_only(&target.unwrap()),
        "build" => {
            // Anything after the program is a C or C++ source built with it.
            let extras: Vec<String> = args.iter().skip(2).cloned().collect();
            nativebuild::build(&target.unwrap(), &extras)
        }
        cmd => {
            // Everything after the program's path belongs to the program,
            // whether or not a subcommand preceded it.
            let file = target.unwrap();
            let at = args.iter().position(|a| *a == file).unwrap_or(0);
            let extra: Vec<String> = args.iter().skip(at + 1).cloned().collect();
            native::set_program_args(extra);
            run_file(&file, cmd == "check", engine)
        }
    }
}

/// Renders a call stack, collapsing runs of identical frames so that a
/// runaway recursion reports one line instead of thousands.
fn format_trace(frames: &[(String, span::Span)], sources: &Sources) -> String {
    let mut out = String::new();
    let mut shown = 0;
    let mut i = 0;
    while i < frames.len() {
        if shown == 12 {
            out.push_str(&format!("  ... and {} more frame(s)\n", frames.len() - i));
            break;
        }
        let (name, at) = &frames[i];
        let mut repeats = 1;
        while i + repeats < frames.len()
            && frames[i + repeats].0 == *name
            && frames[i + repeats].1 == *at
        {
            repeats += 1;
        }
        let where_ = format!("{}:{}:{}", sources.path(at.file), at.line, at.col);
        if repeats > 1 {
            out.push_str(&format!("  in `{}` at {} (x{})\n", name, where_, repeats));
        } else {
            out.push_str(&format!("  in `{}` at {}\n", name, where_));
        }
        shown += 1;
        i += repeats;
    }
    out
}

/// Prints how the program's values are represented: the built-in types, then
/// every class and record, field by field.
///
/// This is the memory model made inspectable. A native backend has to agree
/// with this table byte for byte, so it is worth being able to read it.
fn show_layout(path: &str) -> ExitCode {
    use layout::{builtin_reprs, object_layout, Repr, WORD};

    let mut sources = span::Sources::new();
    let mut program = match loader::load_generating(path, &mut sources) {
        Ok(p) => p,
        Err(d) => {
            eprint!("{}", sources.render("error", &d));
            return ExitCode::FAILURE;
        }
    };

    let mut checker = checker::Checker::new();
    let (errors, _) = checker.check_program(&mut program);
    if !errors.is_empty() {
        for d in &errors {
            eprint!("{}", sources.render("error", d));
        }
        return ExitCode::FAILURE;
    }

    println!("built-in representations");
    println!("  {:<12} {:<22} {:>5}  {:>5}  {}", "type", "as", "size", "align", "notes");
    for (name, repr) in builtin_reprs() {
        let l = repr.layout().expect("a built-in always has a layout");
        let mut notes = Vec::new();
        if repr.is_counted() {
            notes.push("counted");
        }
        if repr.is_c_compatible() {
            notes.push("C-compatible");
        }
        if matches!(repr, Repr::Nullable(_)) {
            notes.push(if repr.has_niche() {
                "null fits in a spare pattern"
            } else {
                "null needs a tag of its own"
            });
        }
        println!(
            "  {:<12} {:<22} {:>5}  {:>5}  {}",
            name,
            repr.to_string(),
            l.size,
            l.align,
            notes.join(", ")
        );
    }

    // The prelude's own records are not what anyone ran this to see.
    let shapes: Vec<_> = checker
        .class_shapes()
        .into_iter()
        .filter(|c| sources.path(c.span.file) != "<prelude>")
        .collect();
    if shapes.is_empty() {
        println!("\nthis program declares no classes or records");
        return ExitCode::SUCCESS;
    }

    for shape in &shapes {
        let laid = object_layout(&shape.name, &shape.fields, shape.generic);
        let kind = if shape.is_record { "record" } else { "class" };
        println!();
        if laid.generic {
            println!(
                "{} {}  —  generic, so one layout per instantiation; \
                 shown with each parameter a pointer",
                kind, laid.name
            );
        } else {
            println!("{} {}", kind, laid.name);
        }
        println!(
            "  {} bytes, align {}, {} of which is padding",
            laid.size,
            laid.align,
            laid.padding()
        );
        println!("  {:>6}  {:>4}  {:<20} {}", "offset", "size", "field", "as");
        println!("  {:>6}  {:>4}  {:<20} {}", 0, WORD, "<reference count>", "usize");
        for f in &laid.fields {
            println!(
                "  {:>6}  {:>4}  {:<20} {}",
                f.offset,
                f.size,
                format!("{}: {}", f.name, f.ty),
                f.repr
            );
        }
    }
    ExitCode::SUCCESS
}

/// One token per line, in a format simple enough that a lexer written in
/// Keal can print the same thing. This is the oracle self-hosting is checked
/// against: the two lexers must agree on every file in the repository.
fn dump_tokens(path: &str) -> ExitCode {
    use lexer::{StrPart, Tok};

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {}", path, e);
            return ExitCode::FAILURE;
        }
    };
    let toks = match lexer::lex(&text, 0) {
        Ok(t) => t,
        Err(d) => {
            println!("error {}:{} {}", d.span.line, d.span.col, d.msg);
            return ExitCode::FAILURE;
        }
    };

    let esc = |s: &str| -> String {
        let mut out = String::new();
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                '\0' => out.push_str("\\0"),
                other => out.push(other),
            }
        }
        out
    };

    let mut out = String::new();
    for t in &toks {
        let head = format!("{}:{}", t.span.line, t.span.col);
        match &t.tok {
            Tok::Int(n) => out.push_str(&format!("{} int {}\n", head, n)),
            Tok::Float(f) => {
                out.push_str(&format!("{} float {}\n", head, runtime::format_float(*f)))
            }
            Tok::Ident(name) => out.push_str(&format!("{} ident {}\n", head, name)),
            Tok::Str(parts) => {
                out.push_str(&format!("{} str", head));
                for p in parts {
                    match p {
                        StrPart::Lit(s) => out.push_str(&format!(" lit({})", esc(s))),
                        StrPart::Interp(src, sp) => out.push_str(&format!(
                            " interp({}:{} {})",
                            sp.line,
                            sp.col,
                            esc(src)
                        )),
                    }
                }
                out.push('\n');
            }
            Tok::Eof => out.push_str(&format!("{} eof\n", head)),
            other => out.push_str(&format!("{} {}\n", head, other.symbol())),
        }
    }
    print!("{}", out);
    ExitCode::SUCCESS
}

/// The parse tree, in the tree format the self-hosted parser reproduces.
fn dump_ast(path: &str) -> ExitCode {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {}", path, e);
            return ExitCode::FAILURE;
        }
    };
    match lexer::lex(&text, 0).and_then(parser::parse) {
        Ok(program) => {
            print!("{}", astdump::dump(&program));
            ExitCode::SUCCESS
        }
        Err(d) => {
            println!("error {}:{} {}", d.span.line, d.span.col, d.msg);
            ExitCode::FAILURE
        }
    }
}

/// The checked tree with every expression's type on it, in a format the
/// self-hosted checker reproduces. Errors come out one per line — compact,
/// deterministic, with the note attached — and the prelude's items are
/// filtered from the dump (its declarations still shape everything shown).
fn dump_types(path: &str) -> ExitCode {
    let mut sources = Sources::new();
    let mut program = match loader::load(path, &mut sources) {
        Ok(p) => p,
        Err(d) => {
            let mut line = format!(
                "error {}:{}:{} {}",
                sources.path(d.span.file),
                d.span.line,
                d.span.col,
                d.msg
            );
            if let Some(note) = &d.note {
                line.push_str(&format!(" -- {}", note));
            }
            println!("{}", line);
            return ExitCode::FAILURE;
        }
    };
    let (errors, warnings) = checker::check(&mut program);
    if !errors.is_empty() {
        for d in &errors {
            let mut line = format!(
                "error {}:{}:{} {}",
                sources.path(d.span.file),
                d.span.line,
                d.span.col,
                d.msg
            );
            if let Some(note) = &d.note {
                line.push_str(&format!(" -- {}", note));
            }
            println!("{}", line);
        }
        return ExitCode::FAILURE;
    }
    for d in &warnings {
        let mut line = format!(
            "warning {}:{}:{} {}",
            sources.path(d.span.file),
            d.span.line,
            d.span.col,
            d.msg
        );
        if let Some(note) = &d.note {
            line.push_str(&format!(" -- {}", note));
        }
        println!("{}", line);
    }
    let prelude: Vec<u32> = (0..sources.len() as u32)
        .filter(|id| sources.path(*id) == "<prelude>")
        .collect();
    print!("{}", astdump::dump_typed(&program, |file| !prelude.contains(&file)));
    ExitCode::SUCCESS
}

/// The generated C on stdout, or — for anything the backend refuses, and for
/// load or check errors — compact one-per-line diagnostics, also on stdout.
/// This is the oracle the self-hosted emitter is held to; `keal emit-c`
/// keeps the human-friendly rendering.
fn dump_cgen(path: &str) -> ExitCode {
    let compact = |sources: &Sources, d: &span::Diag| {
        let mut line = format!(
            "error {}:{}:{} {}",
            sources.path(d.span.file),
            d.span.line,
            d.span.col,
            d.msg
        );
        if let Some(note) = &d.note {
            line.push_str(&format!(" -- {}", note));
        }
        line
    };

    let mut sources = Sources::new();
    let mut program = match loader::load(path, &mut sources) {
        Ok(p) => p,
        Err(d) => {
            println!("{}", compact(&sources, &d));
            return ExitCode::FAILURE;
        }
    };
    let mut checker = checker::Checker::new();
    let (errors, _) = checker.check_program(&mut program);
    if !errors.is_empty() {
        for d in &errors {
            println!("{}", compact(&sources, d));
        }
        return ExitCode::FAILURE;
    }
    for d in &checker.warnings {
        println!("{}", compact_warning(&sources, d));
    }
    match cbackend::emit(&program, &checker.class_shapes()) {
        Ok(c) => {
            print!("{}", c);
            ExitCode::SUCCESS
        }
        Err(diags) => {
            for d in &diags {
                println!("{}", compact(&sources, d));
            }
            ExitCode::FAILURE
        }
    }
}

/// A C header for the program's boundary: the mirror structs its externs
/// share with C, and a prototype for every top-level function whose
/// signature crosses cleanly — so a companion `.c` file compiled by
/// `keal build prog.keal helper.c` can call back into the program.
fn emit_header(path: &str) -> ExitCode {
    use ast::{Item, TypeExpr, TypeExprKind};
    use types::Type;

    let mut sources = Sources::new();
    let mut program = match loader::load_generating(path, &mut sources) {
        Ok(p) => p,
        Err(d) => {
            eprint!("{}", sources.render("error", &d));
            return ExitCode::FAILURE;
        }
    };
    let mut checker = checker::Checker::new();
    let (errors, _) = checker.check_program(&mut program);
    if !errors.is_empty() {
        for d in &errors {
            eprint!("{}", sources.render("error", d));
        }
        return ExitCode::FAILURE;
    }
    let shapes = checker.class_shapes();

    let stem = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "program".into());
    let guard: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect();

    // The scalar C type a written type maps to, when it crosses cleanly.
    let scalar = |te: &TypeExpr| -> Option<&'static str> {
        match &te.kind {
            TypeExprKind::Named { name, args } if args.is_empty() => match name.as_str() {
                "Int" => Some("int64_t"),
                "Float" => Some("double"),
                "Bool" => Some("bool"),
                _ => None,
            },
            _ => None,
        }
    };

    let mut out = String::new();
    out.push_str(&format!(
        "/* Generated by `keal emit-header {}`. Do not edit.
 * The C face of the program's boundary: include this from a companion
 * .c file compiled with `keal build {} companion.c`.
 */
#ifndef KEAL_{}_H
#define KEAL_{}_H

#include <stdbool.h>
#include <stdint.h>

",
        path, path, guard, guard
    ));

    // Mirror structs for every record named in an extern signature — the
    // exact text the generated C defines, so the two agree.
    let mut mirrored: Vec<String> = Vec::new();
    let mut mirror = |te: &TypeExpr, out: &mut String| {
        let name = match &te.kind {
            TypeExprKind::Boundary { inner, .. } => match &inner.kind {
                TypeExprKind::Named { name, args } if args.is_empty() => name.clone(),
                _ => return,
            },
            TypeExprKind::Named { name, args } if args.is_empty() => name.clone(),
            _ => return,
        };
        let Some(shape) = shapes.iter().find(|s| s.name == name) else { return };
        let all_value = !shape.fields.is_empty()
            && shape
                .fields
                .iter()
                .all(|(_, t)| matches!(t, Type::Int | Type::Float | Type::Bool));
        if !all_value || mirrored.contains(&name) {
            return;
        }
        mirrored.push(name.clone());
        out.push_str(&cbackend::mirror_struct_c(&name, &shape.fields));
        out.push('\n');
    };
    for item in &program.items {
        if let Item::Extern(x) = item {
            for p in &x.params {
                if let Some(te) = &p.ty {
                    mirror(te, &mut out);
                }
            }
            if let Some(te) = &x.ret {
                mirror(te, &mut out);
            }
        }
    }

    // Prototypes for the functions C can call back.
    let mut any = false;
    for item in &program.items {
        let Item::Fun(f) = item else { continue };
        if sources.path(f.span.file) == "<prelude>" || !f.type_params.is_empty() {
            continue;
        }
        let ret = match &f.ret {
            None => "void",
            Some(te) => match scalar(te) {
                Some(c) => c,
                None => continue,
            },
        };
        let mut params: Vec<String> = Vec::new();
        let mut ok = true;
        for p in f.params.iter() {
            match p.ty.as_ref().and_then(&scalar) {
                Some(c) => params.push(format!("{} {}", c, p.name)),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let plist = if params.is_empty() { "void".to_string() } else { params.join(", ") };
        out.push_str(&format!("{} k_{}({});
", ret, f.name, plist));
        any = true;
    }
    if !any {
        out.push_str("/* no function of this program crosses the boundary yet */
");
    }
    out.push_str(&format!("
#endif /* KEAL_{}_H */
", guard));
    print!("{}", out);
    ExitCode::SUCCESS
}

/// One warning per line, same shape as the compact errors but labeled.
fn compact_warning(sources: &Sources, d: &crate::span::Diag) -> String {
    let mut line = format!(
        "warning {}:{}:{} {}",
        sources.path(d.span.file),
        d.span.line,
        d.span.col,
        d.msg
    );
    if let Some(note) = &d.note {
        line.push_str(&format!(" -- {}", note));
    }
    line
}

fn run_file(path: &str, check_only: bool, engine: Engine) -> ExitCode {
    let mut sources = Sources::new();

    let mut program = match loader::load_generating(path, &mut sources) {
        Ok(p) => p,
        Err(d) => {
            eprint!("{}", sources.render("error", &d));
            return ExitCode::FAILURE;
        }
    };

    let (errors, warnings) = checker::check(&mut program);
    if !errors.is_empty() {
        for d in &errors {
            eprint!("{}", sources.render("error", d));
        }
        eprintln!(
            "{} error{} found",
            errors.len(),
            if errors.len() == 1 { "" } else { "s" }
        );
        return ExitCode::FAILURE;
    }
    for d in &warnings {
        eprint!("{}", sources.render("warning", d));
    }
    if check_only {
        println!("{}: no errors", path);
        return ExitCode::SUCCESS;
    }

    let outcome = match engine {
        Engine::Ast => interp::Interp::new().run(&program),
        Engine::Bytecode => {
            let unit = match compiler::Compiler::new().compile(&program) {
                Ok(u) => u,
                Err(d) => {
                    eprint!("{}", sources.render("error", &d));
                    return ExitCode::FAILURE;
                }
            };
            vm::Vm::new().run(&unit).map(|_| ())
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprint!("{}", sources.render("runtime error", &e.diag));
            eprint!("{}", format_trace(&e.frames, &sources));
            ExitCode::FAILURE
        }
    }
}
