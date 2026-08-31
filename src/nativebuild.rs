//! Driving a native build: emit C, then hand it to a C compiler.
//!
//! The C compiler is found in the environment rather than assumed, and its
//! output is passed through unchanged when it complains — a failure there is
//! a bug in what this emitted, and hiding it would help nobody.

use std::path::Path;
use std::process::{Command, ExitCode};

use crate::{cbackend, checker, loader, span::Sources};

/// Type-checks and emits C, or reports why it cannot.
fn compile_with(path: &str, audit: bool) -> Result<String, ExitCode> {
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

    cbackend::emit_with(&program, &checker.class_shapes(), audit).map_err(|diags| {
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

pub fn emit_only(path: &str, audit: bool) -> ExitCode {
    match compile_with(path, audit) {
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
/// The C driver to use: `CC` when it is set, otherwise the first name on
/// this machine that answers.
///
/// `cc` is the Unix convention and does not exist on Windows, where a
/// MinGW `gcc` or an LLVM `clang` is what a developer has. Looking past
/// the first name is the difference between "no C compiler" and a working
/// `keal build` there.
pub fn c_driver() -> String {
    driver("CC", &["cc", "gcc", "clang"])
}

pub fn cxx_driver() -> String {
    driver("CXX", &["c++", "g++", "clang++"])
}

fn driver(var: &str, candidates: &[&str]) -> String {
    if let Ok(named) = std::env::var(var) {
        return named;
    }
    for name in candidates {
        if command_for(name).arg("--version").output().is_ok() {
            return name.to_string();
        }
    }
    // Nothing answered: name the conventional one, so the message a caller
    // prints says what it looked for.
    candidates[0].to_string()
}

/// A driver name as a command. `CC` is allowed to carry arguments — people
/// set `CC="zig cc"`, and on Windows they reach for it precisely because
/// there is no `cc` — so the first word is the program and the rest are
/// arguments it always gets.
pub fn command_for(driver: &str) -> Command {
    let mut parts = driver.split_whitespace();
    let program = parts.next().unwrap_or(driver);
    let mut cmd = Command::new(program);
    for arg in parts {
        cmd.arg(arg);
    }
    cmd
}

/// What to say when nothing answered. On Windows the likely truth is
/// specific enough to be worth naming: the toolchain a Rust install brings
/// is MSVC, and MSVC is the one compiler this runtime cannot use.
pub fn no_compiler_advice() -> Vec<String> {
    let mut out = Vec::new();
    if cfg!(windows) && Command::new("cl").output().is_ok() {
        out.push(
            "`cl.exe` (MSVC) is installed, and it cannot build the Keal runtime: \
             the overflow checks are GCC/Clang builtins"
                .to_string(),
        );
        out.push(
            "install MinGW-w64 (a POSIX-threads build, which actors need) or LLVM clang \
             targeting mingw32, and put it on PATH"
                .to_string(),
        );
        return out;
    }
    out.push("set CC to a C compiler, or install one".to_string());
    if cfg!(windows) {
        out.push(
            "on Windows that means MinGW-w64 (a POSIX-threads build, which actors need) \
             or LLVM clang targeting mingw32"
                .to_string(),
        );
    }
    out
}

/// `audit` is the build's switch, not the program's: a binary cannot grow
/// counters after it is compiled, so the audit is asked for here where the
/// interpreters take the same question from `KEAL_AUDIT`.
pub fn build(path: &str, extras: &[String], audit: bool) -> ExitCode {
    let c = match compile_with(path, audit) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned());
    let base = stem.unwrap_or_else(|| "a.out".to_string());
    // Windows runs `program.exe` and nothing else; every other system runs
    // whatever the file is called.
    let out = if cfg!(windows) { format!("{}.exe", base) } else { base.clone() };
    let csrc = format!("{}.c", base);

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

    let cc = c_driver();
    let cxx = cxx_driver();

    // The generated file is C11 whatever else is on the line; a C++ driver
    // would reject its compound literals, so it is compiled to an object
    // first and only the link is shared.
    let obj = format!("{}.o", base);
    // `-pthread` is the actor scheduler's, and nothing else's: the emitted
    // C says whether it wants one. A program without actors then builds on
    // a toolchain that has no threads library at all.
    let threaded = c.contains("#define KEAL_ACTORS");
    let mut compile_cmd = command_for(&cc);
    compile_cmd.args(["-O2", "-std=c11"]);
    if threaded {
        compile_cmd.arg("-pthread");
    }
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
            // Name what was looked for, not what happened to be tried first:
            // `cc` is a Unix convention, and a Windows developer reading that
            // it could not run has no `cc` to look for and none to install.
            if std::env::var("CC").is_ok() {
                eprintln!("error: cannot run `{}`, which `CC` names: {}", cc, e);
            } else {
                eprintln!("error: no C compiler found — tried `cc`, `gcc`, `clang`");
            }
            for line in no_compiler_advice() {
                eprintln!("  = note: {}", line);
            }
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
        let sobj = format!("{}-x{}.o", base, i);
        let driver = if is_cpp(extra) { &cxx } else { &cc };
        let mut sc = command_for(driver);
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
    let mut cmd = command_for(linker);
    cmd.args(["-O2", "-o", &out]);
    if threaded {
        cmd.arg("-pthread");
    }
    for o in &objs {
        cmd.arg(o);
    }
    for f in &compile_flags {
        cmd.arg(f);
    }
    for extra in rest {
        cmd.arg(extra);
    }
    // The runtime calls `pow` and `floor`. Where libm is a library of its
    // own the link has to say so; on Windows it is part of the C runtime and
    // asking for it fails.
    if !cfg!(windows) {
        cmd.arg("-lm");
    }
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
