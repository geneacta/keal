//! The `keal` command-line driver: module loading, running, checking, REPL.

mod ast;
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
    keal emit-c <file.keal>   print the C a native build would compile
    keal build <file.keal> [more.c more.cpp ...]
                              compile to a native executable, together with
                              any C or C++ sources the externs need
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
    let (command, target) = match args.as_slice() {
        [] => ("repl", None),
        [one] if one == "repl" => ("repl", None),
        [one] if one == "version" || one == "--version" || one == "-V" => ("version", None),
        [one] if one == "help" || one == "--help" || one == "-h" => ("help", None),
        [one] => ("run", Some(one.clone())),
        [cmd, file, ..]
            if matches!(cmd.as_str(), "run" | "check" | "layout" | "emit-c" | "build") =>
        {
            (
                match cmd.as_str() {
                    "run" => "run",
                    "check" => "check",
                    "layout" => "layout",
                    "emit-c" => "emit-c",
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
        "emit-c" => nativebuild::emit_only(&target.unwrap()),
        "build" => {
            // Anything after the program is a C or C++ source built with it.
            let extras: Vec<String> = args.iter().skip(2).cloned().collect();
            nativebuild::build(&target.unwrap(), &extras)
        }
        cmd => run_file(&target.unwrap(), cmd == "check", engine),
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
    let mut program = match loader::load(path, &mut sources) {
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

fn run_file(path: &str, check_only: bool, engine: Engine) -> ExitCode {
    let mut sources = Sources::new();

    let mut program = match loader::load(path, &mut sources) {
        Ok(p) => p,
        Err(d) => {
            eprint!("{}", sources.render("error", &d));
            return ExitCode::FAILURE;
        }
    };

    let errors = checker::check(&mut program);
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
