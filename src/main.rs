//! The `keal` command-line driver: module loading, running, checking, REPL.

mod ast;
mod builtins;
mod bytecode;
mod checker;
mod compiler;
mod interp;
mod lexer;
mod loader;
mod native;
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
        [cmd, file] if cmd == "run" || cmd == "check" => {
            (if cmd == "run" { "run" } else { "check" }, Some(file.clone()))
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
