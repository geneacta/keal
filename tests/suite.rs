//! End-to-end tests driving the `keal` binary.
//!
//! Every program runs on **both** engines — the bytecode VM and the
//! tree-walking evaluator — and the two must agree. The evaluator is the
//! reference implementation: it is simple enough to read as a specification,
//! so any disagreement is a bug in the VM until shown otherwise.
//!
//! * `tests/programs/**` are self-checking Keal programs: they use `assert`
//!   and must exit 0 while printing nothing.
//! * `tests/errors/*.keal` must fail `keal check`; their diagnostics are
//!   compared against a `.expected` snapshot.
//! * `tests/runtime/*.keal` must pass the checker and fail at run time.
//!
//! Run with `UPDATE_EXPECT=1 cargo test` to rewrite the snapshots.

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_keal");

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

struct Output {
    stdout: String,
    stderr: String,
    success: bool,
}

/// Runs the binary from the crate root so that the paths in diagnostics are
/// the relative ones written in the snapshots.
fn keal(args: &[&str]) -> Output {
    let out = Command::new(BIN)
        .args(args)
        .current_dir(root())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {}: {}", BIN, e));
    Output {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        success: out.status.success(),
    }
}

/// Every `.keal` file directly inside `dir`, sorted for a stable order.
fn keal_files(dir: &str) -> Vec<PathBuf> {
    let full = root().join(dir);
    let Ok(entries) = std::fs::read_dir(&full) else {
        panic!("missing test directory: {}", full.display());
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "keal").unwrap_or(false))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .keal files in {}", full.display());
    files
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root()).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// Compares against a snapshot, or rewrites it when `UPDATE_EXPECT` is set.
fn check_snapshot(source: &Path, actual: &str) {
    let expected_path = source.with_extension("expected");
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        std::fs::write(&expected_path, actual).expect("cannot write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap_or_else(|_| {
        panic!(
            "missing snapshot {}\nrun `UPDATE_EXPECT=1 cargo test` to create it\n--- actual ---\n{}",
            expected_path.display(),
            actual
        )
    });
    assert_eq!(
        expected,
        actual,
        "\nsnapshot mismatch for {}\nrun `UPDATE_EXPECT=1 cargo test` to update",
        relative(source)
    );
}

/// The two engines, named as the command line spells them.
const ENGINES: [&str; 2] = ["--vm", "--ast"];

#[test]
fn programs_pass_their_own_assertions() {
    for file in keal_files("tests/programs") {
        let path = relative(&file);
        for engine in ENGINES {
            let out = keal(&[engine, &path]);
            assert!(out.success, "{} failed on {}:\n{}", path, engine, out.stderr);
            assert!(
                out.stdout.is_empty(),
                "{} printed unexpected output on {}:\n{}",
                path,
                engine,
                out.stdout
            );
        }
    }
}

#[test]
fn modules_are_loaded_once() {
    for engine in ENGINES {
        let out = keal(&[engine, "tests/programs/modules/main.keal"]);
        assert!(out.success, "module test failed on {}:\n{}", engine, out.stderr);
        assert!(out.stdout.is_empty(), "module test printed:\n{}", out.stdout);
    }
}

#[test]
fn examples_run_successfully() {
    for file in keal_files("examples") {
        let path = relative(&file);
        for engine in ENGINES {
            let out = keal(&[engine, &path]);
            assert!(out.success, "example {} failed on {}:\n{}", path, engine, out.stderr);
        }
    }
}

/// The heart of the arrangement: whatever a program prints, and whatever it
/// fails with, must not depend on which engine ran it.
#[test]
fn both_engines_agree() {
    let mut files = keal_files("examples");
    files.extend(keal_files("tests/programs"));
    files.extend(keal_files("tests/runtime"));
    files.push(root().join("tests/programs/modules/main.keal"));

    for file in files {
        let path = relative(&file);
        let vm = keal(&["--vm", &path]);
        let ast = keal(&["--ast", &path]);
        assert_eq!(vm.stdout, ast.stdout, "engines printed differently for {}", path);
        assert_eq!(vm.stderr, ast.stderr, "engines failed differently for {}", path);
        assert_eq!(vm.success, ast.success, "engines disagreed on success for {}", path);
    }
}

#[test]
fn type_errors_match_snapshots() {
    for file in keal_files("tests/errors") {
        let path = relative(&file);
        let out = keal(&["check", &path]);
        assert!(!out.success, "{} was expected to fail the checker", path);
        check_snapshot(&file, &out.stderr);
    }
}

#[test]
fn runtime_errors_match_snapshots() {
    for file in keal_files("tests/runtime") {
        let path = relative(&file);
        let checked = keal(&["check", &path]);
        assert!(
            checked.success,
            "{} should type-check but fail at run time:\n{}",
            path, checked.stderr
        );
        let out = keal(&[&path]);
        assert!(!out.success, "{} was expected to fail at run time", path);
        check_snapshot(&file, &out.stderr);
    }
}

#[test]
fn cli_reports_missing_files() {
    let out = keal(&["run", "does/not/exist.keal"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("cannot read"),
        "unhelpful message for a missing file: {}",
        out.stderr
    );
}

#[test]
fn version_is_printed() {
    let out = keal(&["version"]);
    assert!(out.success);
    assert!(out.stdout.starts_with("keal "), "unexpected version output: {}", out.stdout);
}
