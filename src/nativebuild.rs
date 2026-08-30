//! Driving a native build: emit C, then hand it to a C compiler.
//!
//! The C compiler is found in the environment rather than assumed, and its
//! output is passed through unchanged when it complains — a failure there is
//! a bug in what this emitted, and hiding it would help nobody.

use std::path::Path;
use std::process::{Command, ExitCode};

use crate::{cbackend, checker, loader, span::Sources};

/// Type-checks and emits C, or reports why it cannot.
fn compile(path: &str) -> Result<String, ExitCode> {
    let mut sources = Sources::new();
    let mut program = match loader::load_generating(path, &mut sources) {
        Ok(p) => p,
        Err(d) => {
            eprint!("{}", sources.render("error", &d));
            return Err(ExitCode::FAILURE);
        }
    };

    let mut checker = checker::Checker::new();
    let (errors, _) = checker.check_program(&mut program);
    if !errors.is_empty() {
        for d in &errors {
            eprint!("{}", sources.render("error", d));
        }
        return Err(ExitCode::FAILURE);
    }

    cbackend::emit(&program, &checker.class_shapes()).map_err(|diags| {
        for d in &diags {
            eprint!("{}", sources.render("error", d));
        }
        eprintln!(
            "{} construct{} the C backend does not cover yet",
            diags.len(),
            if diags.len() == 1 { "" } else { "s" }
        );
        ExitCode::FAILURE
    })
}

pub fn emit_only(path: &str) -> ExitCode {
    match compile(path) {
        Ok(c) => {
            print!("{}", c);
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Builds an executable. `extras` fall into three kinds:
///
/// * **sources** (`.c`, `.cpp`, `.cc`, `.cxx`, `.C`) — compiled alongside,
///   where the implementations behind `extern fun` live when a `native`
///   block is not enough. Any C++ among them makes `c++` the linker.
/// * **link inputs** (`.a`, `.so`, `.dylib`, `.o`, `-l...`, `-L...`) —
///   handed to the link step untouched. This is how a Rust `staticlib`, a
///   Go `c-archive`, or any prebuilt C library joins the program.
/// * **compile flags** (`-I...`, `-D...`) — applied when compiling the
///   generated C and the extra sources, and passed to the link line too,
///   where the sources are actually built.
pub fn build(path: &str, extras: &[String]) -> ExitCode {
    let c = match compile(path) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned());
    let out = stem.unwrap_or_else(|| "a.out".to_string());
    let csrc = format!("{}.c", out);

    if let Err(e) = std::fs::write(&csrc, &c) {
        eprintln!("error: cannot write `{}`: {}", csrc, e);
        return ExitCode::FAILURE;
    }

    let is_cpp = |p: &str| {
        Path::new(p)
            .extension()
            .map(|e| matches!(e.to_str(), Some("cpp" | "cc" | "cxx" | "C")))
            .unwrap_or(false)
    };
    let any_cpp = extras.iter().any(|p| is_cpp(p));
    let compile_flags: Vec<&String> = extras
        .iter()
        .filter(|a| a.starts_with("-I") || a.starts_with("-D"))
        .collect();
    let link_line: Vec<&String> = extras
        .iter()
        .filter(|a| !(a.starts_with("-I") || a.starts_with("-D")))
        .collect();

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let cxx = std::env::var("CXX").unwrap_or_else(|_| "c++".to_string());

    // The generated file is C11 whatever else is on the line; a C++ driver
    // would reject its compound literals, so it is compiled to an object
    // first and only the link is shared.
    let obj = format!("{}.o", out);
    let mut compile_cmd = Command::new(&cc);
    compile_cmd.args(["-O2", "-std=c11", "-pthread"]);
    for f in &compile_flags {
        compile_cmd.arg(f);
    }
    let compiled = compile_cmd.args(["-c", "-o", &obj, &csrc]).status();
    match compiled {
        Ok(s) if s.success() => {}
        Ok(_) => {
            eprintln!(
                "error: `{}` failed on the generated C, which is left at `{}`",
                cc, csrc
            );
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("error: cannot run `{}`: {}", cc, e);
            eprintln!("  = note: set CC to a C compiler, or install one");
            return ExitCode::FAILURE;
        }
    }

    // Extra sources each get their own object under their own compiler:
    // C stays C11 under `cc`, C++ goes to `c++` — mixing them on one
    // driver line only earns warnings.
    let mut objs = vec![obj.clone()];
    let mut rest: Vec<&String> = Vec::new();
    for (i, extra) in link_line.into_iter().enumerate() {
        let is_source = Path::new(extra)
            .extension()
            .map(|e| matches!(e.to_str(), Some("c" | "cpp" | "cc" | "cxx" | "C")))
            .unwrap_or(false);
        if !is_source {
            rest.push(extra);
            continue;
        }
        let sobj = format!("{}-x{}.o", out, i);
        let driver = if is_cpp(extra) { &cxx } else { &cc };
        let mut sc = Command::new(driver);
        sc.arg("-O2");
        if is_cpp(extra) {
            sc.arg("-std=c++17");
        } else {
            sc.arg("-std=c11");
        }
        for f in &compile_flags {
            sc.arg(f);
        }
        let ok = sc.args(["-c", "-o", &sobj, extra]).status();
        match ok {
            Ok(s) if s.success() => objs.push(sobj),
            Ok(_) => {
                eprintln!("error: `{}` failed compiling `{}`", driver, extra);
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("error: cannot run `{}`: {}", driver, e);
                return ExitCode::FAILURE;
            }
        }
    }

    let linker = if any_cpp { &cxx } else { &cc };
    let mut cmd = Command::new(linker);
    cmd.args(["-O2", "-pthread", "-o", &out]);
    for o in &objs {
        cmd.arg(o);
    }
    for f in &compile_flags {
        cmd.arg(f);
    }
    for extra in rest {
        cmd.arg(extra);
    }
    cmd.arg("-lm");
    match cmd.status() {
        Ok(s) if s.success() => {
            let _ = std::fs::remove_file(&csrc);
            for o in &objs {
                let _ = std::fs::remove_file(o);
            }
            println!("{}", out);
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!(
                "error: `{}` failed linking; the generated C is left at `{}`",
                linker, csrc
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("error: cannot run `{}`: {}", linker, e);
            ExitCode::FAILURE
        }
    }
}
