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
    let mut program = match loader::load(path, &mut sources) {
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

/// Builds an executable. `extras` are C or C++ sources compiled alongside —
/// where the implementations behind `extern fun` live when a `native` block
/// is not enough. Any C++ among them makes `c++` the linker, so its runtime
/// is present; the generated core stays C either way.
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

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let cxx = std::env::var("CXX").unwrap_or_else(|_| "c++".to_string());

    // The generated file is C11 whatever else is on the line; a C++ driver
    // would reject its compound literals, so it is compiled to an object
    // first and only the link is shared.
    let obj = format!("{}.o", out);
    let compiled = Command::new(&cc)
        .args(["-O2", "-std=c11", "-c", "-o", &obj, &csrc])
        .status();
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

    let linker = if any_cpp { &cxx } else { &cc };
    let mut cmd = Command::new(linker);
    cmd.args(["-O2", "-o", &out, &obj]);
    for extra in extras {
        cmd.arg(extra);
    }
    match cmd.status() {
        Ok(s) if s.success() => {
            let _ = std::fs::remove_file(&csrc);
            let _ = std::fs::remove_file(&obj);
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
