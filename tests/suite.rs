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

/// The C driver this machine has, the way `keal build` looks for it: `CC`
/// when set, then `cc`, `gcc`, `clang`. A Windows machine has the last two
/// and not the first, and its tests should run rather than skip.
fn c_driver() -> String {
    if let Ok(named) = std::env::var("CC") {
        return named;
    }
    for name in ["cc", "gcc", "clang"] {
        if Command::new(name).arg("--version").output().is_ok() {
            return name.to_string();
        }
    }
    "cc".to_string()
}

/// Where the JDK is, or `None`.
///
/// `JAVA_HOME` first, because it is the portable answer and the one a
/// Windows or Linux machine will have set. `/usr/libexec/java_home` is a
/// macOS helper and nothing else: asking for it on any other system found
/// no JDK at all, however many were installed.
fn java_home() -> Option<String> {
    if let Ok(h) = std::env::var("JAVA_HOME") {
        if !h.trim().is_empty() && Path::new(h.trim()).join("include").exists() {
            return Some(h.trim().to_string());
        }
    }
    let out = Command::new("/usr/libexec/java_home").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// The subdirectory a JDK keeps `jni_md.h` in.
///
/// `jni.h` includes it by bare name, and every JDK files it under the
/// platform: `darwin`, `linux`, `win32`. The JDK's address is portable now;
/// its layout has to be spelled out.
fn jni_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    }
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

/// Two modules may declare the same names. The importing file says which
/// it means — bare for the unaliased one, through the alias for the other —
/// and both engines must agree that they are two different things.
#[test]
fn namespaces_keep_two_modules_apart() {
    for engine in ENGINES {
        let out = keal(&[engine, "tests/programs/namespaces/main.keal"]);
        assert!(out.success, "namespace test failed on {}:\n{}", engine, out.stderr);
        assert!(out.stdout.is_empty(), "namespace test printed:\n{}", out.stdout);
    }
}

/// `import "dep:geometry/shapes.keal"` reads `.keal/deps/` beside the
/// nearest `keal.toml`. The dependency here is committed rather than
/// fetched, which is the point: what is on disk is what is read, so this
/// needs neither network nor git.
#[test]
fn dependencies_are_imported_from_the_project_root() {
    for engine in ENGINES {
        let out = keal(&[engine, "tests/deps/main.keal"]);
        assert!(out.success, "dependency test failed on {}:\n{}", engine, out.stderr);
        assert!(out.stdout.is_empty(), "dependency test printed:\n{}", out.stdout);
    }
}

/// A `dep:` import that nothing has fetched says so, and says what to run.
#[test]
fn a_missing_dependency_says_to_fetch() {
    let out = keal(&["check", "tests/deps/missing.keal"]);
    assert!(!out.success);
    assert!(
        out.stderr.contains("keal fetch"),
        "unhelpful message for a missing dependency: {}",
        out.stderr
    );
}

/// `keal fetch` end to end, against a git repository made on the spot:
/// clone at a tag, import through `dep:`, run. Skipped where git is not
/// installed, since nothing else in the compiler needs it.
#[test]
fn fetch_puts_a_dependency_where_an_import_finds_it() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: no `git`");
        return;
    }
    let dir = std::env::temp_dir().join("keal-fetch-test");
    let _ = std::fs::remove_dir_all(&dir);
    let dep = dir.join("upstream");
    let project = dir.join("project");
    std::fs::create_dir_all(&dep).expect("cannot make the upstream directory");
    std::fs::create_dir_all(&project).expect("cannot make the project directory");
    std::fs::write(
        dep.join("shapes.keal"),
        "public record Circle(val r: Float)\npublic fun area(c: Circle): Float { 3.0 * c.r * c.r }\n",
    )
    .expect("cannot write the dependency");

    let git = |args: &[&str], at: &Path| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(at)
            .output()
            .expect("cannot run git");
        assert!(ok.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&ok.stderr));
    };
    git(&["init", "-q", "."], &dep);
    git(&["config", "user.email", "t@example.com"], &dep);
    git(&["config", "user.name", "Test"], &dep);
    git(&["add", "-A"], &dep);
    git(&["commit", "-qm", "shapes"], &dep);
    git(&["tag", "v1.0.0"], &dep);

    std::fs::write(
        project.join("keal.toml"),
        format!(
            "[package]\nname = \"p\"\nversion = \"0.1.0\"\n\n[dependencies]\ngeometry = {{ git = \"{}\", tag = \"v1.0.0\" }}\n",
            dep.display()
        ),
    )
    .expect("cannot write the manifest");
    std::fs::write(
        project.join("main.keal"),
        "import \"dep:geometry/shapes.keal\"\nprintln(area(Circle(2.0)))\n",
    )
    .expect("cannot write the program");

    let fetched = Command::new(BIN)
        .arg("fetch")
        .current_dir(&project)
        .output()
        .expect("cannot run keal fetch");
    assert!(
        fetched.status.success(),
        "keal fetch failed:\n{}",
        String::from_utf8_lossy(&fetched.stderr)
    );

    for engine in ENGINES {
        let ran = Command::new(BIN)
            .args([engine, "main.keal"])
            .current_dir(&project)
            .output()
            .expect("cannot run the program");
        assert!(
            ran.status.success(),
            "the fetched dependency did not run on {}:\n{}",
            engine,
            String::from_utf8_lossy(&ran.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "12.0");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The cycle audit: with `KEAL_AUDIT` set, a program says at exit what it
/// left behind, by type. Counting is the whole of it — nothing here
/// diagnoses a cycle, it reports the evidence one leaves. Both engines
/// must count the same objects, and a program without the variable set
/// must print exactly what it printed before the audit existed.
#[test]
fn the_audit_names_what_outlived_the_program() {
    let path = "tests/audit/cycle.keal";
    let quiet = keal(&["--vm", path]);
    assert!(quiet.success);
    assert!(
        !quiet.stderr.contains("audit:"),
        "the audit spoke without being asked:\n{}",
        quiet.stderr
    );

    for engine in ENGINES {
        let out = Command::new(BIN)
            .args([engine, path])
            .current_dir(root())
            .env("KEAL_AUDIT", "1")
            .output()
            .expect("cannot run the audit");
        assert!(out.status.success());
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            err.contains("2 object(s) outlived the program")
                && err.contains("1 Item")
                && err.contains("1 Owner"),
            "{} did not report the cycle:\n{}",
            engine,
            err
        );
        // The pair without a back edge dies, and says so on the way out.
        let printed = String::from_utf8_lossy(&out.stdout);
        assert!(printed.contains("owner 3 died"), "the acyclic pair did not run its deinit");
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
    let cc = c_driver();
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
            .args(["-O2", "-std=c11", "-pthread", "-o"])
            .arg(&bin)
            .arg(&csrc)
            // The runtime calls `pow` and `floor`; where libm is a library of
            // its own the link has to say so, and where it is part of libc
            // this asks for nothing.
            .arg("-lm")
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

/// The threaded scheduler under ThreadSanitizer: the mesh program — eight
/// actors fanning echoes at each other while posting into one outbox —
/// builds with `-fsanitize=thread` and must come back clean, five runs in
/// a row. Skipped when no C compiler is installed, and when this compiler
/// cannot link the sanitizer runtime, so the suite stays green on machines
/// that cannot run the check rather than pretending they did.
#[test]
fn actors_are_clean_under_thread_sanitizer() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    let path = "tests/native/actor-mesh.keal";
    let emitted = keal(&["emit-c", path]);
    assert!(emitted.success, "{} did not emit C:\n{}", path, emitted.stderr);

    let dir = std::env::temp_dir().join("keal-actor-tsan");
    std::fs::create_dir_all(&dir).expect("cannot make a build directory");
    let csrc = dir.join("out.c");
    let bin = dir.join("out");
    std::fs::write(&csrc, &emitted.stdout).expect("cannot write the generated C");

    let built = Command::new(&cc)
        .args(["-O2", "-std=c11", "-pthread", "-fsanitize=thread", "-o"])
        .arg(&bin)
        .arg(&csrc)
        .arg("-lm")
        .output()
        .expect("cannot run the C compiler");
    if !built.status.success() {
        eprintln!("skipping: `{}` cannot build with -fsanitize=thread", cc);
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    for _ in 0..5 {
        let out = Command::new(&bin).output().expect("cannot run the built binary");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            !stderr.contains("ThreadSanitizer"),
            "the thread sanitizer reported a race:\n{}",
            stderr
        );
        assert!(out.status.success(), "the sanitized binary failed:\n{}", stderr);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "total 1117\n",
            "the sanitized binary printed the wrong total"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
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
    let cc = c_driver();
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
    files.extend(keal_files("tests/native-extern"));
    files.extend(keal_files("tests/selfhost"));
    files.extend(keal_files("tests/selfhost/errors"));
    files.push(root().join("lib/jvm.keal"));
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
    files.extend(keal_files("tests/native-extern"));
    files.extend(keal_files("tests/selfhost"));
    files.extend(keal_files("tests/selfhost/errors"));
    files.extend(keal_files("tests/selfhost/parse-errors"));
    files.push(root().join("lib/jvm.keal"));
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
    files.extend(keal_files("tests/native-extern"));
    files.extend(keal_files("tests/selfhost"));
    files.extend(keal_files("tests/selfhost/errors"));
    files.extend(keal_files("tests/selfhost/parse-errors"));
    files.extend(keal_files("tests/selfhost/type-errors"));
    files.push(root().join("lib/jvm.keal"));
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
    files.extend(keal_files("tests/native-extern"));
    files.extend(keal_files("tests/selfhost"));
    files.extend(keal_files("tests/selfhost/errors"));
    files.extend(keal_files("tests/selfhost/parse-errors"));
    files.extend(keal_files("tests/selfhost/type-errors"));
    files.push(root().join("lib/jvm.keal"));
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

/// The bootstrap: `keal build selfhost/cbackend.keal` compiles the
/// self-hosted compiler to a native binary, and that binary must behave as
/// the Rust oracle does — on ordinary programs, on programs that fail, and
/// on its own source, where its output must be the very C it was built
/// from. A compiler written in Keal, compiled by itself, at a fixed point.
#[test]
fn the_compiler_compiles_itself() {
    // It compiles through C, so it needs the compiler `keal build` needs.
    // Skipped rather than failed where there is none, like every other test
    // that reaches for one.
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    let dir = root().join("target").join("bootstrap-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the bootstrap dir");

    let built = Command::new(BIN)
        .args(["build", &root().join("selfhost/cbackend.keal").to_string_lossy()])
        .current_dir(&dir)
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the self-hosted compiler did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let native = dir.join("cbackend");

    let cases = [
        "tests/native/core.keal",
        "tests/native/builtins.keal",
        "tests/selfhost/type-errors/te01.keal",
        "tests/selfhost/parse-errors/perr16.keal",
        "selfhost/cbackend.keal",
    ];
    for case in cases {
        let oracle = keal(&["cgen", case]);
        let out = Command::new(&native)
            .arg(case)
            .current_dir(root())
            .output()
            .expect("cannot run the bootstrapped compiler");
        assert_eq!(
            oracle.stdout,
            String::from_utf8_lossy(&out.stdout),
            "the bootstrapped compiler disagrees on {}",
            case
        );
        assert_eq!(
            oracle.status_success(),
            out.status.success(),
            "the bootstrapped compiler disagrees on whether {} compiles",
            case
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `keal emit-header` prints the C face of a program's boundary: the mirror
/// structs its externs share with C, and a `k_` prototype for every function
/// that crosses cleanly.
#[test]
fn emit_header_matches_snapshot() {
    let out = keal(&["emit-header", "tests/native-extern/boundary.keal"]);
    assert!(out.success, "emit-header failed:\n{}", out.stderr);
    let expected_path = root().join("tests/native-extern/boundary.h.expected");
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        std::fs::write(&expected_path, &out.stdout).expect("cannot write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&expected_path)
        .expect("missing snapshot; run UPDATE_EXPECT=1 cargo test");
    assert_eq!(expected, out.stdout, "the generated header changed");
}

/// `keal bindgen` turns a C header into extern declarations, binding only
/// what crosses the boundary exactly and skipping the rest with a reason.
#[test]
fn bindgen_matches_snapshot() {
    let out = keal(&["bindgen", "tests/bindgen/sample.h"]);
    assert!(out.success, "bindgen failed:\n{}", out.stderr);
    let expected_path = root().join("tests/bindgen/sample.h.expected");
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        std::fs::write(&expected_path, &out.stdout).expect("cannot write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&expected_path)
        .expect("missing snapshot; run UPDATE_EXPECT=1 cargo test");
    assert_eq!(expected, out.stdout, "the generated bindings changed");
}

/// The whole Rust/Go-shaped path in miniature: `bindgen` a header, implement
/// it in a **static library**, and `keal build prog.keal libsample.a -I...`
/// links it in. Everything the generated bindings promise must run.
#[test]
fn bindgen_and_link_inputs_work_end_to_end() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    let ar = std::env::var("AR").unwrap_or_else(|_| "ar".to_string());
    if Command::new(&ar).arg("--version").output().is_err() {
        eprintln!("skipping: no archiver found as `{}`", ar);
        return;
    }

    let dir = root().join("target").join("bindgen-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the bindgen test dir");

    // The implementation of the clean half of tests/bindgen/sample.h.
    std::fs::write(
        dir.join("sample.c"),
        r#"#include "tests/bindgen/sample.h"
#include <ctype.h>
#include <stdlib.h>
#include <string.h>
int64_t add64(int64_t a, int64_t b) { return a + b; }
long long triple(long long n) { return n * 3; }
double scale(double x, double factor) { return x * factor; }
bool flag_of(int64_t n) { return n % 2 == 0; }
int64_t count_vowels(const char *text) {
    int64_t n = 0;
    for (; *text; text++) {
        char c = (char)tolower((unsigned char)*text);
        if (c == 'a' || c == 'e' || c == 'i' || c == 'o' || c == 'u') { n++; }
    }
    return n;
}
char *shout(const char *text) {
    size_t n = strlen(text);
    char *out = (char *)malloc(n + 2);
    for (size_t i = 0; i < n; i++) { out[i] = (char)toupper((unsigned char)text[i]); }
    out[n] = '!';
    out[n + 1] = '\0';
    return out;
}
void reset(void) {}
void tick() {}
double vec2_dot(Keal_Vec2 a, Keal_Vec2 b) { return a.x * b.x + a.y * b.y; }
Keal_Vec2 vec2_scale(Keal_Vec2 v, double k) { return (Keal_Vec2){ v.x * k, v.y * k }; }
int64_t unnamed_params(int64_t a, double b) { return a + (int64_t)b; }
"#,
    )
    .expect("cannot write sample.c");

    let compiled = Command::new(&cc)
        .current_dir(&dir)
        .args(["-O2", "-std=c11", "-c", "-o", "sample.o", "sample.c"])
        .arg(format!("-I{}", root().display()))
        .status()
        .expect("cannot run cc");
    assert!(compiled.success(), "sample.c did not compile");
    let archived = Command::new(&ar)
        .current_dir(&dir)
        .args(["rcs", "libsample.a", "sample.o"])
        .status()
        .expect("cannot run ar");
    assert!(archived.success(), "libsample.a was not created");

    // The bindings module comes straight from bindgen.
    let bindings = keal(&["bindgen", "tests/bindgen/sample.h"]);
    assert!(bindings.success);
    std::fs::write(dir.join("bindings.keal"), &bindings.stdout)
        .expect("cannot write bindings.keal");

    // But Vec2 is Keal's to declare: the record the mirror struct reflects.
    std::fs::write(
        dir.join("prog.keal"),
        r#"public record Vec2(val x: Float, val y: Float)
import "./bindings.keal"
println(add64(40, triple(1)))
println(scale(2.5, 4.0))
println(flag_of(8))
println(count_vowels("static library"))
println(shout("linked"))
reset()
tick()
val v = vec2_scale(Vec2(3.0, 4.0), 2.0)
println(v)
println(vec2_dot(v, Vec2(0.5, 0.25)))
println(unnamed_params(40, 2.9))
"#,
    )
    .expect("cannot write prog.keal");

    let built = Command::new(BIN)
        .current_dir(&dir)
        .args(["build", "prog.keal", "libsample.a"])
        .arg(format!("-I{}", root().display()))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "keal build with link inputs failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = Command::new(dir.join("prog")).output().expect("cannot run the binary");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "43\n10.0\ntrue\n4\nLINKED!\nVec2(x=6.0, y=8.0)\n5.0\n42\n",
        "the linked program printed the wrong thing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The JVM gateway: `java.time.LocalDate` driven from a natively compiled
/// Keal program through lib/jvm.keal. Skipped when no JDK is around.
/// `keal jbind` on saved `javap` output: deterministic, JDK-free, and the
/// generated module must satisfy the type checker as written.
#[test]
fn jbind_matches_snapshot_and_typechecks() {
    let out = keal(&[
        "jbind",
        "--jvm",
        "../../lib/jvm.keal",
        "tests/jbind/localdate.javap",
        "tests/jbind/uuid.javap",
    ]);
    assert!(out.success, "jbind failed:\n{}", out.stderr);
    let expected_path = root().join("tests/jbind/expected.keal");
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        std::fs::write(&expected_path, &out.stdout).expect("cannot write snapshot");
    } else {
        let expected = std::fs::read_to_string(&expected_path)
            .expect("missing snapshot; run UPDATE_EXPECT=1 cargo test");
        assert_eq!(expected, out.stdout, "the generated wrappers changed");
    }
    let checked = keal(&["check", "tests/jbind/expected.keal"]);
    assert!(
        checked.status_success(),
        "the generated wrappers do not type-check:\n{}",
        checked.stderr
    );
}

/// The full jbind road under a real JDK: generate `java.time` wrappers with
/// live `javap`, build a native program against them, and run it.
#[test]
fn jbind_works_end_to_end() {
    let Some(jh) = java_home() else {
        eprintln!("skipping: no JDK found (set JAVA_HOME)");
        return;
    };
    if !Path::new(&jh).join("include/jni.h").exists() {
        eprintln!("skipping: JDK without JNI headers");
        return;
    }

    let dir = root().join("target").join("jbind-e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the jbind test dir");

    let generated = keal(&[
        "jbind",
        "--jvm",
        root().join("lib/jvm.keal").to_str().unwrap(),
        "java.time.LocalDate",
        "java.time.DayOfWeek",
    ]);
    assert!(generated.success, "jbind failed under a live JDK:\n{}", generated.stderr);
    std::fs::write(dir.join("timegen.keal"), &generated.stdout).expect("cannot write the module");
    std::fs::write(
        dir.join("main.keal"),
        r#"import "timegen.keal"
jvmStart("")
val d = localDateOf(2026, 1, 1)
val later = d.plusDays(58)
println(later.toString())
val dow = later.getDayOfWeek()
println(dow.toString())
println(later.isLeapYear().toString())
println(later.getYear().toString())
println(later.lengthOfMonth().toString())
dow.free()
later.free()
d.free()
"#,
    )
    .expect("cannot write the program");

    let built = Command::new(BIN)
        .current_dir(&dir)
        .arg("build")
        .arg("main.keal")
        .arg(format!("-I{}/include", jh))
        .arg(format!("-I{}/include/{}", jh, jni_platform()))
        .arg(format!("-L{}/lib/server", jh))
        .arg("-ljvm")
        .arg(format!("-Wl,-rpath,{}/lib/server", jh))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the jbind wrappers did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = Command::new(dir.join("main")).output().expect("cannot run the binary");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "2026-02-28\nSATURDAY\nfalse\n2026\n28\n",
        "the jbind wrappers printed the wrong thing:\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The endpoint of the interop plan: `import java.time.LocalDate` with no
/// path. The build generates the `.jbind/` cache through `javap` and links
/// a native binary; the dump commands never generate, so this stays here,
/// JDK-gated, and the corpora stay pure.
#[test]
fn import_sugar_works_end_to_end() {
    let Some(jh) = java_home() else {
        eprintln!("skipping: no JDK found (set JAVA_HOME)");
        return;
    };
    if !Path::new(&jh).join("include/jni.h").exists() {
        eprintln!("skipping: JDK without JNI headers");
        return;
    }

    let dir = root().join("target").join("sugar-e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the sugar test dir");
    std::fs::write(
        dir.join("main.keal"),
        r#"import java.time.LocalDate, java.time.DayOfWeek
jvmStart("")
val d = localDateOf(2026, 1, 1)
val later = d.plusDays(58)
println(later.toString())
val dow = later.getDayOfWeek()
println(dow.toString())
dow.free()
later.free()
d.free()
"#,
    )
    .expect("cannot write the program");

    let built = Command::new(BIN)
        .current_dir(&dir)
        .arg("build")
        .arg("main.keal")
        .arg(format!("-I{}/include", jh))
        .arg(format!("-I{}/include/{}", jh, jni_platform()))
        .arg(format!("-L{}/lib/server", jh))
        .arg("-ljvm")
        .arg(format!("-Wl,-rpath,{}/lib/server", jh))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the sugar import did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(
        dir.join(".jbind/java.time.LocalDate+java.time.DayOfWeek.keal").exists(),
        "the build did not fill the .jbind cache"
    );
    let ran = Command::new(dir.join("main")).output().expect("cannot run the binary");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "2026-02-28\nSATURDAY\n",
        "the sugar import printed the wrong thing:\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The interop payoff of native `try`: a Java exception, caught in a
/// native binary, with the program carrying on.
#[test]
fn java_exceptions_are_catchable_natively() {
    let Some(jh) = java_home() else {
        eprintln!("skipping: no JDK found (set JAVA_HOME)");
        return;
    };
    if !Path::new(&jh).join("include/jni.h").exists() {
        eprintln!("skipping: JDK without JNI headers");
        return;
    }

    let dir = root().join("target").join("jexc-e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the jexc test dir");
    std::fs::write(
        dir.join("main.keal"),
        r#"import java.time.LocalDate
jvmStart("")
val good = localDateOf(2026, 2, 28)
println(good.toString())
try {
    val bad = localDateOf(2026, 13, 1)
    println(bad.toString())
} catch (e) {
    println("caught: " + e.take(26))
}
println("still running")
good.free()
"#,
    )
    .expect("cannot write the program");

    let built = Command::new(BIN)
        .current_dir(&dir)
        .arg("build")
        .arg("main.keal")
        .arg(format!("-I{}/include", jh))
        .arg(format!("-I{}/include/{}", jh, jni_platform()))
        .arg(format!("-L{}/lib/server", jh))
        .arg("-ljvm")
        .arg(format!("-Wl,-rpath,{}/lib/server", jh))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the program did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = Command::new(dir.join("main")).output().expect("cannot run the binary");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "2026-02-28\ncaught: of threw: java.time.DateTi\nstill running\n",
        "the caught Java exception went wrong:\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jvm_gateway_works_end_to_end() {
    let Some(jh) = java_home() else {
        eprintln!("skipping: no JDK found (set JAVA_HOME)");
        return;
    };
    if !Path::new(&jh).join("include/jni.h").exists() {
        eprintln!("skipping: JDK without JNI headers");
        return;
    }

    let dir = root().join("target").join("jvm-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the jvm test dir");

    let built = Command::new(BIN)
        .current_dir(&dir)
        .arg("build")
        .arg(root().join("examples/interop/java/localdate.keal"))
        .arg(format!("-I{}/include", jh))
        .arg(format!("-I{}/include/{}", jh, jni_platform()))
        .arg(format!("-L{}/lib/server", jh))
        .arg("-ljvm")
        .arg(format!("-Wl,-rpath,{}/lib/server", jh))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the JVM gateway did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = Command::new(dir.join("localdate")).output().expect("cannot run the binary");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "2026-02-28\nSATURDAY\nfalse\n20512\n6765\n",
        "the JVM gateway printed the wrong thing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Java from an actor thread: the gateway must attach the thread to the
/// JVM lazily at its first call and detach it when the actor ends — a
/// JNIEnv is only valid on the thread it was handed to, so using main's
/// from an actor is undefined behavior, not a slow path. Skipped without
/// a JDK, like the other gateway tests.
#[test]
fn jvm_calls_work_from_actor_threads() {
    let Some(jh) = java_home() else {
        eprintln!("skipping: no JDK found (set JAVA_HOME)");
        return;
    };
    if !Path::new(&jh).join("include/jni.h").exists() {
        eprintln!("skipping: JDK without JNI headers");
        return;
    }

    let dir = root().join("target").join("jvm-actor-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the jvm actor test dir");

    let built = Command::new(BIN)
        .current_dir(&dir)
        .arg("build")
        .arg(root().join("examples/interop/java/actordate.keal"))
        .arg(format!("-I{}/include", jh))
        .arg(format!("-I{}/include/{}", jh, jni_platform()))
        .arg(format!("-L{}/lib/server", jh))
        .arg("-ljvm")
        .arg(format!("-Wl,-rpath,{}/lib/server", jh))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the actor JVM program did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = Command::new(dir.join("actordate")).output().expect("cannot run the binary");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "2026-01-01 is a THURSDAY\n2026-01-02 is a FRIDAY\n2026-01-03 is a SATURDAY\n",
        "the actor asked Java and got the wrong answer:\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `keal doc` renders the compiler's own signatures with their `///`
/// comments; the snapshot keeps the page shape honest.
#[test]
fn kealdoc_matches_snapshot() {
    let out = keal(&["doc", "tests/doc/sample.keal"]);
    assert!(out.success, "keal doc failed:\n{}", out.stderr);
    let expected_path = root().join("tests/doc/sample.html.expected");
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        std::fs::write(&expected_path, &out.stdout).expect("cannot write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&expected_path)
        .expect("missing snapshot; run UPDATE_EXPECT=1 cargo test");
    assert_eq!(expected, out.stdout, "the generated documentation changed");
}

/// The site's tour tells the reader that every snippet on it is a real
/// program and every output beside it is what that program prints. This is
/// what makes the sentence true. Skipped without Python, which is what reads
/// the page's own source of snippets.
#[test]
fn the_site_tour_prints_what_it_promises() {
    let python = "python3";
    if Command::new(python).arg("--version").output().is_err() {
        eprintln!("skipping: no `{}` to read the tour with", python);
        return;
    }
    let out = Command::new(python)
        .current_dir(root())
        .arg("site/checktour.py")
        .arg(BIN)
        .output()
        .expect("cannot run the tour check");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
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
