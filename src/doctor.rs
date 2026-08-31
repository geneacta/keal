//! `keal doctor` — what the interop toolchains are on this machine, next to
//! the versions the repository's own tests last verified.
//!
//! Keal does not vendor compilers: a C compiler, a JDK or a Go toolchain is
//! hundreds of megabytes of platform-specific binaries, and every OS ships
//! or packages them better than a language repo can. What the repo *can*
//! promise is the exact versions its interop suite was green against — this
//! table — and this command says at a glance where a machine differs.
//! Only `cargo` (and a C compiler for `keal build`) are required; every
//! other row is optional and unlocks one interop path.

use std::process::{Command, ExitCode};

/// The versions the interop test suite was last verified against, updated
/// whenever the suite runs green on a new toolchain.
const KNOWN_GOOD: &[(&str, &str)] = &[
    ("cc", "Apple clang 21.0.0"),
    ("cargo", "rustc 1.98.0"),
    ("go", "go 1.27.0"),
    ("javac", "JDK 23.0.2"),
    ("kotlinc", "kotlinc 2.4.10"),
];

struct Probe {
    tool: &'static str,
    args: &'static [&'static str],
    unlocks: &'static str,
}

const PROBES: &[Probe] = &[
    Probe { tool: "cc", args: &["--version"], unlocks: "keal build (required for native)" },
    Probe { tool: "cargo", args: &["--version"], unlocks: "building the toolchain (required)" },
    Probe { tool: "go", args: &["version"], unlocks: "Go interop (c-archive)" },
    Probe { tool: "javac", args: &["-version"], unlocks: "JVM interop (Java)" },
    Probe { tool: "javap", args: &["-version"], unlocks: "keal jbind / import java.*" },
    Probe { tool: "kotlinc", args: &["-version"], unlocks: "Kotlin interop" },
];

pub fn run() -> ExitCode {
    println!("keal doctor — the interop toolchains on this machine\n");
    let mut missing_required = false;
    // The C driver is whichever this machine actually has: `cc` on Unix,
    // more often `gcc` or `clang` on Windows. Reporting the one that will
    // be used beats reporting the one that is conventional.
    let cc = crate::nativebuild::c_driver();
    for p in PROBES {
        let tool = if p.tool == "cc" { cc.as_str() } else { p.tool };
        let mut found = crate::nativebuild::command_for(tool).args(p.args).output();
        // Windows ships some of these as `.bat` shims, and Rust's PATH
        // search appends only `.exe` — it does not read PATHEXT. A tool
        // reported missing because of that would be a lie.
        if cfg!(windows) && found.is_err() {
            for ext in [".bat", ".cmd"] {
                let shim = format!("{}{}", tool, ext);
                let retry = Command::new(&shim).args(p.args).output();
                if retry.is_ok() {
                    found = retry;
                    break;
                }
            }
        }
        let line = match &found {
            Ok(out) if out.status.success() => {
                let all = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                all.lines().next().unwrap_or("").trim().to_string()
            }
            _ => String::new(),
        };
        let known = KNOWN_GOOD
            .iter()
            .find(|(t, _)| *t == p.tool)
            .map(|(_, v)| *v);
        if line.is_empty() {
            let required = p.unlocks.contains("required");
            if required {
                missing_required = true;
            }
            println!(
                "  {:8} MISSING{}  — {}",
                tool,
                if required { " (required)" } else { "          " },
                p.unlocks
            );
        } else {
            println!("  {:8} {}", tool, line);
            if let Some(k) = known {
                println!("  {:8}   verified against: {}", "", k);
            }
            println!("  {:8}   unlocks: {}\n", "", p.unlocks);
            continue;
        }
        println!();
    }
    println!(
        "A different version is not an error — these are the versions the\n\
         interop tests were last green against. `cargo test --release` on\n\
         this machine is the real answer."
    );
    if missing_required {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
