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

pub fn build(path: &str) -> ExitCode {
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

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let status = Command::new(&cc)
        .args(["-O2", "-std=c11", "-o", &out, &csrc])
        .status();

    match status {
        Ok(s) if s.success() => {
            let _ = std::fs::remove_file(&csrc);
            println!("{}", out);
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!(
                "error: `{}` failed on the generated C, which is left at `{}`",
                cc, csrc
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("error: cannot run `{}`: {}", cc, e);
            eprintln!("  = note: set CC to a C compiler, or install one");
            ExitCode::FAILURE
        }
    }
}
