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

impl Output {
    fn status_success(&self) -> bool {
        self.success
    }
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

/// How values are laid out is a decision, not an accident, so it is pinned
/// down: a change to any representation shows up as a diff here.
#[test]
fn layouts_match_snapshots() {
    for file in keal_files("tests/layout") {
        let path = relative(&file);
        let out = keal(&["layout", &path]);
        assert!(out.success, "{} failed to lay out:\n{}", path, out.stderr);
        check_snapshot(&file, &out.stdout);
    }
}

/// The native backend must agree with the interpreters, not merely compile.
///
/// This emits C, hands it to a real C compiler, runs the binary, and compares
/// its output with both other engines. It is skipped when no C compiler is
/// installed rather than failing, since one is not needed to work on the rest
/// of the language.
#[test]
fn native_agrees_with_the_interpreters() {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }

    for file in keal_files("tests/native") {
        let path = relative(&file);
        let emitted = keal(&["emit-c", &path]);
        assert!(emitted.success, "{} did not emit C:\n{}", path, emitted.stderr);

        let dir = std::env::temp_dir().join(format!(
            "keal-native-{}",
            file.file_stem().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&dir).expect("cannot make a build directory");
        let csrc = dir.join("out.c");
        let bin = dir.join("out");
        std::fs::write(&csrc, &emitted.stdout).expect("cannot write the generated C");

        let built = Command::new(&cc)
            .args(["-O2", "-std=c11", "-o"])
            .arg(&bin)
            .arg(&csrc)
            .output()
            .expect("cannot run the C compiler");
        assert!(
            built.status.success(),
            "the C generated for {} did not compile:\n{}",
            path,
            String::from_utf8_lossy(&built.stderr)
        );

        let native = Command::new(&bin).output().expect("cannot run the built binary");
        let native_out = String::from_utf8_lossy(&native.stdout).into_owned();

        for engine in ENGINES {
            let interpreted = keal(&[engine, &path]);
            assert!(interpreted.success, "{} failed on {}", path, engine);
            assert_eq!(
                native_out, interpreted.stdout,
                "native output differs from {} for {}",
                engine, path
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A construct the backend does not cover must be named, not mis-compiled.
#[test]
fn the_native_backend_says_what_it_cannot_compile() {
    for file in keal_files("tests/native-unsupported") {
        let path = relative(&file);
        let out = keal(&["emit-c", &path]);
        assert!(!out.success, "{} was expected to be refused", path);
        check_snapshot(&file, &out.stderr);
    }
}

/// Interop programs build with the real C and C++ compilers and print what
/// the snapshot says. Skipped without a C compiler, like the native tests.
#[test]
fn extern_programs_build_and_run() {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    for file in keal_files("tests/native-extern") {
        let path = relative(&file);
        let companion = file.with_extension("cpp");
        let dir = std::env::temp_dir().join("keal-extern-test");
        std::fs::create_dir_all(&dir).expect("cannot make a build directory");

        let mut cmd = Command::new(BIN);
        cmd.current_dir(&dir).arg("build").arg(root().join(&path));
        if companion.exists() {
            cmd.arg(&companion);
        }
        let built = cmd.output().expect("cannot run keal build");
        assert!(
            built.status.success(),
            "{} did not build:\n{}",
            path,
            String::from_utf8_lossy(&built.stderr)
        );
        let exe = dir.join(file.file_stem().unwrap());
        let ran = Command::new(&exe).output().expect("cannot run the built binary");
        check_snapshot(&file, &String::from_utf8_lossy(&ran.stdout));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The self-hosted lexer must print exactly what the Rust one prints, for
/// every file in the repository and for every way lexing can fail. This is
/// the first plank of self-hosting: when the whole compiler is Keal, this
/// test is how each piece earns its way in.
#[test]
fn selfhosted_lexer_agrees_with_the_oracle() {
    let mut files = keal_files("tests/programs");
    files.extend(keal_files("examples"));
    files.extend(keal_files("tests/native"));
    files.extend(keal_files("tests/selfhost"));
    files.extend(keal_files("tests/selfhost/errors"));
    files.push(root().join("selfhost/lexer.keal"));
    files.push(root().join("src/prelude.keal"));

    for file in files {
        let path = relative(&file);
        let oracle = keal(&["tokens", &path]);
        let mine = keal(&["--vm", "selfhost/lexer.keal", &path]);
        assert_eq!(
            oracle.stdout, mine.stdout,
            "the lexers disagree on {}",
            path
        );
        assert_eq!(
            oracle.status_success(),
            mine.status_success(),
            "the lexers disagree on whether {} lexes",
            path
        );
    }
}

/// The self-hosted parser must print exactly the tree the Rust one prints —
/// spans, error messages and exit codes included — for every file in the
/// repository, valid or not. Second plank of self-hosting.
#[test]
fn selfhosted_parser_agrees_with_the_oracle() {
    let mut files = keal_files("tests/programs");
    files.extend(keal_files("examples"));
    files.extend(keal_files("tests/native"));
    files.extend(keal_files("tests/selfhost"));
    files.extend(keal_files("tests/selfhost/errors"));
    files.extend(keal_files("tests/selfhost/parse-errors"));
    files.push(root().join("selfhost/lexer.keal"));
    files.push(root().join("selfhost/lexing.keal"));
    files.push(root().join("selfhost/parser.keal"));
    files.push(root().join("src/prelude.keal"));

    for file in files {
        let path = relative(&file);
        let oracle = keal(&["ast", &path]);
        let mine = keal(&["--vm", "selfhost/parser.keal", &path]);
        assert_eq!(
            oracle.stdout, mine.stdout,
            "the parsers disagree on {}",
            path
        );
        assert_eq!(
            oracle.status_success(),
            mine.status_success(),
            "the parsers disagree on whether {} parses",
            path
        );
    }
}

/// The self-hosted checker must print exactly the typed tree the Rust one
/// prints — inferred types, generic instantiations, operator rewrites — or
/// exactly its diagnostics, sorted the same, notes included. Third plank of
/// self-hosting, and the corpus includes the checker checking itself.
#[test]
fn selfhosted_checker_agrees_with_the_oracle() {
    let mut files = keal_files("tests/programs");
    files.extend(keal_files("examples"));
    files.extend(keal_files("tests/native"));
    files.extend(keal_files("tests/selfhost"));
    files.extend(keal_files("tests/selfhost/errors"));
    files.extend(keal_files("tests/selfhost/parse-errors"));
    files.extend(keal_files("tests/selfhost/type-errors"));
    files.push(root().join("selfhost/checker.keal"));
    files.push(root().join("src/prelude.keal"));

    for file in files {
        let path = relative(&file);
        let oracle = keal(&["types", &path]);
        let mine = keal(&["--vm", "selfhost/checker.keal", &path]);
        assert_eq!(
            oracle.stdout, mine.stdout,
            "the checkers disagree on {}",
            path
        );
        assert_eq!(
            oracle.status_success(),
            mine.status_success(),
            "the checkers disagree on whether {} checks",
            path
        );
    }
}

/// The self-hosted C emitter must produce exactly the C the Rust backend
/// produces — mangled names, temp numbering, ownership releases, refusal
/// diagnostics and exit codes included. Fourth plank of self-hosting: a
/// native compiler written in the language it compiles.
#[test]
fn selfhosted_emitter_agrees_with_the_oracle() {
    let mut files = keal_files("tests/programs");
    files.extend(keal_files("examples"));
    files.extend(keal_files("tests/native"));
    files.extend(keal_files("tests/selfhost"));
    files.extend(keal_files("tests/selfhost/errors"));
    files.extend(keal_files("tests/selfhost/parse-errors"));
    files.extend(keal_files("tests/selfhost/type-errors"));
    files.push(root().join("selfhost/cbackend.keal"));
    files.push(root().join("src/prelude.keal"));

    for file in files {
        let path = relative(&file);
        let oracle = keal(&["cgen", &path]);
        let mine = keal(&["--vm", "selfhost/cbackend.keal", &path]);
        assert_eq!(
            oracle.stdout, mine.stdout,
            "the emitters disagree on {}",
            path
        );
        assert_eq!(
            oracle.status_success(),
            mine.status_success(),
            "the emitters disagree on whether {} compiles",
            path
        );
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
