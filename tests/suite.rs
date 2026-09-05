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
    if let Ok(out) = Command::new("/usr/libexec/java_home").output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }
    // `javac` on the PATH, resolved to the JDK it lives in.
    //
    // Without this there was no answer on Linux at all: `JAVA_HOME` or
    // `/usr/libexec/java_home`, and the second is macOS's. A Debian or
    // Ubuntu machine with a working JDK — which does not set `JAVA_HOME`,
    // because the package does not — skipped all four interop tests while
    // printing `ok`. That is a test standing down because it would rather
    // not, which is the one reason rule 6 does not allow.
    let exe = if cfg!(windows) { "javac.exe" } else { "javac" };
    let sep = if cfg!(windows) { ';' } else { ':' };
    for dir in std::env::var("PATH").ok()?.split(sep) {
        let candidate = Path::new(dir).join(exe);
        if !candidate.exists() {
            continue;
        }
        // `/usr/bin/javac` is a symlink into the JDK; the real path is what
        // names the home, two levels above `bin/javac`.
        let Ok(real) = std::fs::canonicalize(&candidate) else { continue };
        let Some(home) = real.parent().and_then(|b| b.parent()) else { continue };
        if home.join("include").exists() {
            return Some(home.to_string_lossy().into_owned());
        }
    }
    None
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

/// How a JNI program links, per platform.
///
/// macOS and Linux keep `libjvm` under `lib/server` and are told where to
/// find it again at run time with an rpath. Windows keeps the import
/// library at `lib/jvm.lib` — which `-ljvm` resolves — and has no rpath at
/// all: PE finds `jvm.dll` through `PATH`, which is what `jvm_run` hands it.
fn jni_link_args(jh: &str) -> Vec<String> {
    if cfg!(windows) {
        return vec![format!("-L{}/lib", jh), "-ljvm".to_string()];
    }
    vec![
        format!("-L{}/lib/server", jh),
        "-ljvm".to_string(),
        format!("-Wl,-rpath,{}/lib/server", jh),
    ]
}

/// A command that will find the JVM's own libraries when it runs.
///
/// On Windows both directories are needed: `bin\\server` has `jvm.dll`, and
/// `bin` has the runtime libraries `jvm.dll` itself loads. Elsewhere the
/// rpath in the binary has already said this, and nothing is added.
fn jvm_run(program: PathBuf, jh: &str) -> Command {
    let mut cmd = Command::new(program);
    if cfg!(windows) {
        let existing = std::env::var("PATH").unwrap_or_default();
        cmd.env(
            "PATH",
            format!("{jh}\\bin\\server;{jh}\\bin;{existing}", jh = jh, existing = existing),
        );
    }
    cmd
}

/// Copies a file, or a directory and everything under it.
fn copy_into(from: &Path, to: &Path) {
    if from.is_dir() {
        std::fs::create_dir_all(to).expect("cannot make a directory");
        for entry in std::fs::read_dir(from).expect("cannot read a directory") {
            let path = entry.expect("cannot read an entry").path();
            let name = path.file_name().expect("a path has a name");
            copy_into(&path, &to.join(name));
        }
        return;
    }
    std::fs::copy(from, to).expect("cannot copy a file");
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
        "public record Circle(val r: Float)\npublic func area(c: Circle): Float { 3.0 * c.r * c.r }\n",
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
    // The two interpreters, on every shape the audit is meant to see: a
    // cycle, a cycle broken by `weak`, and actors holding one another.
    for path in ["tests/audit/cycle.keal", "tests/audit/reachable.keal",
                 "tests/audit/closure-cycle.keal",
                 "tests/native/weak.keal",
                 "tests/native/actors.keal", "tests/native/actor-mesh.keal"] {
        let mut reports = Vec::new();
        for engine in ENGINES {
            let out = Command::new(BIN)
                .args([engine, path])
                .current_dir(root())
                .env("KEAL_AUDIT", "1")
                .output()
                .expect("cannot run the audit");
            assert!(out.status.success(), "{} failed on {}", path, engine);
            reports.push(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        assert_eq!(reports[0], reports[1], "the engines disagree about {}", path);
        assert!(reports[0].contains("audit:"), "no audit for {}", path);
        // That the audit spoke is not that it said the right thing. This one
        // exists to tell two `Holder`s apart — the closure that captured
        // `this` and the one that read the field into a local first — so the
        // assertion has to name the answer, not the fact of an answer. A
        // check that passes whatever the audit concludes is green forever
        // and attests nothing.
        if path == "tests/audit/closure-cycle.keal" {
            assert!(
                reports[0].contains("2 object(s) outlived the program")
                    && reports[0].contains("— a cycle"),
                "two of the three holders make a cycle and the audit must \
                 say so:\n{}",
                reports[0]
            );
            // Two, not three: the file builds three holders the same way,
            // and they differ only in what their closure holds. One names
            // `this`, one names a local that IS the object, and one reads
            // the field into a local first — so `3 Holder` would mean the
            // audit had stopped telling them apart, and `1 Holder` would
            // mean the rule had been read as being about `this` alone.
            assert!(
                reports[0].contains("2 Holder") && !reports[0].contains("3 Holder"),
                "exactly two of the three holders are cycles:\n{}",
                reports[0]
            );
        }
    }

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
        // And the verdict, which is the point: nothing here is held by a
        // top-level binding, so all of it is named as a cycle.
        assert!(
            err.contains("2 of them are reachable from no top-level binding"),
            "{} did not call the cycle a cycle:\n{}",
            engine,
            err
        );
    }

    // The other half of the rule: a program that leaves both kinds behind
    // must name each for what it is, and the three engines must agree
    // about which is which.
    for engine in ENGINES {
        let out = Command::new(BIN)
            .args([engine, "tests/audit/reachable.keal"])
            .current_dir(root())
            .env("KEAL_AUDIT", "1")
            .output()
            .expect("cannot run the audit");
        assert!(out.status.success());
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            err.contains("11 object(s) outlived the program")
                && err.contains("2 of them are reachable from no top-level binding")
                && err.contains("the rest are held by a top-level binding"),
            "{} did not tell the cycle from the roots:\n{}",
            engine,
            err
        );
    }
}

/// `main` runs, and a `main` that cannot be run is said so rather than left
/// sitting there.
///
/// The silence this ends was real: before this, a file whose whole program
/// was inside `proc main()` compiled cleanly, printed nothing and exited 0 —
/// which is what anyone arriving from C, Java, Rust or Go writes first.
#[test]
fn a_main_runs_and_a_malformed_one_is_refused() {
    // The call is appended by the loader, so every engine inherits it. The
    // program's own assertions pin the order — top level first, `main` last
    // — but they cannot prove `main` ran at all, since a `main` that never
    // runs never asserts. The exit code below is what proves that.
    for engine in ENGINES {
        let out = keal(&[engine, "tests/programs/main.keal"]);
        assert!(out.success, "`main` did not run under {}:\n{}", engine, out.stderr);
    }

    // `func main(): Int` — the Int is the exit code, as in C, and a code the
    // top level cannot produce is the proof that `main` itself ran.
    let dir = std::env::temp_dir().join("keal-main-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a directory");
    std::fs::write(dir.join("code.keal"), "func main(): Int { return 7 }\n").unwrap();
    for engine in ENGINES {
        let out = Command::new(BIN)
            .args([engine, "code.keal"])
            .current_dir(&dir)
            .output()
            .expect("cannot run keal");
        assert_eq!(out.status.code(), Some(7), "the exit code is not `main`'s under {}", engine);
    }

    // A shape that cannot be a `main` is a message, not a silent no-op.
    for (src, wanted) in [
        ("func main(): String { return \"x\" }\n", "must return `Int`"),
        ("proc main(a: Int) {}\n", "takes no parameters"),
    ] {
        std::fs::write(dir.join("bad.keal"), src).unwrap();
        let out = Command::new(BIN)
            .args(["run", "bad.keal"])
            .current_dir(&dir)
            .output()
            .expect("cannot run keal");
        assert!(!out.status.success(), "`{}` was accepted", src.trim());
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(said.contains(wanted), "wrong message for `{}`:\n{}", src.trim(), said);
    }

    // A module's `main` is not the program's: only the entry file's runs.
    std::fs::write(dir.join("lib.keal"), "public proc main() { println(\"library\") }\n").unwrap();
    std::fs::write(dir.join("app.keal"), "import \"./lib.keal\"\nprintln(\"app\")\n").unwrap();
    let out = Command::new(BIN)
        .args(["run", "app.keal"])
        .current_dir(&dir)
        .output()
        .expect("cannot run keal");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "app\n", "an imported `main` ran");
}

/// The audit is a verdict, and a program must not be able to silence it by
/// ending the ordinary way.
///
/// `exit` leaves through the C library and never comes back, so everything
/// the audit would have said at the end of `main` went unsaid — including
/// for the call the loader now appends for `func main(): Int`. The verdict
/// is emitted before any `exit` written among the top-level statements,
/// where the roots it marks from are still in scope; below that, `exit` is
/// refused under the audit rather than quietly excused.
#[test]
fn no_exit_can_silence_the_audit() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    let dir = std::env::temp_dir().join("keal-audit-exit");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a directory");

    let cycle = "class Node { var next: Node? = null }\nval a = Node()\nval b = Node()\n";
    for (name, src) in [
        // The exit the loader appends for a `func main(): Int`.
        ("viamain", format!("{}proc tie() {{ a.next = b; b.next = a }}\nfunc main(): Int {{ tie()\n return 0 }}\n", cycle)),
        // And one written by hand among the top-level statements.
        ("byhand", format!("{}a.next = b\nb.next = a\nexit(0)\n", cycle)),
    ] {
        let file = dir.join(format!("{}.keal", name));
        std::fs::write(&file, src).unwrap();
        let built = Command::new(BIN)
            .current_dir(&dir)
            .args(["--audit", "build"])
            .arg(&file)
            .output()
            .expect("cannot run keal build");
        assert!(built.status.success(), "{} did not build:\n{}", name,
                String::from_utf8_lossy(&built.stderr));
        let ran = Command::new(dir.join(name)).output().expect("cannot run the binary");
        let said = String::from_utf8_lossy(&ran.stderr);
        assert!(said.contains("audit:"), "`exit` silenced the audit in {}:\n{}", name, said);
    }

    // Below the top level the roots are out of scope, so the audit says so
    // rather than reporting a verdict it cannot stand behind.
    let file = dir.join("deep.keal");
    std::fs::write(&file, "proc bail() { exit(2) }\nval x = [1, 2]\nbail()\n").unwrap();
    let built = Command::new(BIN)
        .current_dir(&dir)
        .args(["--audit", "build"])
        .arg(&file)
        .output()
        .expect("cannot run keal build");
    assert!(!built.status.success(), "`exit` inside a function was audited anyway");
    let said = String::from_utf8_lossy(&built.stderr);
    assert!(said.contains("`exit` inside a function under `--audit`"), "wrong refusal:\n{}", said);

    // And without the audit it is an ordinary program.
    let built = Command::new(BIN).current_dir(&dir).arg("build").arg(&file).output().unwrap();
    assert!(built.status.success(), "`exit` in a function stopped compiling:\n{}",
            String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(dir.join("deep")).output().unwrap();
    assert_eq!(ran.status.code(), Some(2));
}

/// Every program in the corpus, compiled — the third consumer.
///
/// `tests/programs` had two: the tree-walker and the bytecode VM. The C
/// backend, which is where nearly every defect of the last week has been,
/// never saw it — `tests/native` is a separate and much smaller corpus. So
/// the corpus attested that two engines agreed with each other, which is a
/// weaker thing than it reads as.
///
/// Asking the third engine about all 33 at once turned up nine defects in an
/// afternoon: a nullable scalar compared against a plain one emitting a C
/// struct comparison, a method used as a value emitting a field access, a
/// `return this` handing back a reference it never took, a `weak` release
/// freeing its own header underneath itself, a celled-variable map with no
/// frame, a capture analysis that let a global outrank the local shadowing
/// it — and one test that was asserting an interleaving the language says it
/// does not promise.
///
/// A program the backend REFUSES is fine and is counted: refusing by name is
/// the backend working. What must never happen is emitting C that does not
/// compile, or a program that runs and disagrees with the interpreters.
#[test]
fn programs_compile_and_agree_natively() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    let dir = std::env::temp_dir().join("keal-programs-native");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a build directory");

    let mut refused = 0;
    let mut agreed = 0;
    for file in keal_files("tests/programs") {
        let path = relative(&file);
        let stem = Path::new(&path).file_stem().unwrap().to_string_lossy().into_owned();
        let out_path = dir.join(&stem);
        let built = Command::new(BIN)
            .args(["build", &path])
            .arg("-o")
            .arg(&out_path)
            .current_dir(root())
            .output()
            .expect("cannot run keal build");
        if !built.status.success() {
            let said = String::from_utf8_lossy(&built.stderr);
            // The backend saying what it cannot do is correct behaviour.
            assert!(
                said.contains("cannot compile"),
                "{} emitted C that does not compile:\n{}",
                path,
                said
            );
            refused += 1;
            continue;
        }
        // Run it where the interpreters run it: some of these read and write
        // files under `target/`.
        let native = Command::new(&out_path)
            .current_dir(root())
            .output()
            .expect("cannot run the compiled program");
        let interpreted = keal(&["run", &path]);
        assert_eq!(
            String::from_utf8_lossy(&native.stdout),
            interpreted.stdout,
            "{} prints something different when compiled",
            path
        );
        assert_eq!(
            native.status.code(),
            Some(if interpreted.success { 0 } else { 1 }),
            "{} ends differently when compiled:\n{}",
            path,
            String::from_utf8_lossy(&native.stderr)
        );
        agreed += 1;
    }
    assert!(agreed > 20, "only {} programs compiled; the corpus should mostly build", agreed);
    eprintln!("{} programs agree with the interpreters, {} refused by name", agreed, refused);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The audit under `keal build --audit`: the same question the interpreters
/// answer from the environment, answered by a compiled binary in the same
/// words. A binary cannot grow counters after it is compiled, which is why
/// this one is asked at build time; the report has to be identical anyway,
/// or the three engines disagree about what a program left behind.
#[test]
fn the_native_audit_says_what_the_interpreters_say() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    let dir = std::env::temp_dir().join("keal-audit-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a build directory");
    // Five shapes, not one: a plain cycle, a cycle a `weak` edge breaks,
    // the cycle a closure that captured `this` makes, and two actor programs
    // whose objects hold each other. Covering only the first is how a
    // disagreement between engines went unnoticed once.
    for name in ["tests/audit/cycle.keal", "tests/audit/reachable.keal",
                 "tests/audit/closure-cycle.keal",
                 "tests/native/weak.keal",
                 "tests/native/actors.keal", "tests/native/actor-mesh.keal"] {
    let src = root().join(name);

    let built = Command::new(BIN)
        .current_dir(&dir)
        .args(["--audit", "build"])
        .arg(&src)
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the audited program did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let stem = Path::new(name).file_stem().unwrap().to_string_lossy().into_owned();
    let ran = Command::new(dir.join(&stem)).output().expect("cannot run the binary");
    let native = String::from_utf8_lossy(&ran.stderr).into_owned();

    for engine in ENGINES {
        let interpreted = Command::new(BIN)
            .args([engine, &relative(&src)])
            .current_dir(root())
            .env("KEAL_AUDIT", "1")
            .output()
            .expect("cannot run the audit");
        assert_eq!(
            native,
            String::from_utf8_lossy(&interpreted.stderr),
            "the native audit and {} disagree about what outlived the program",
            engine
        );
    }
    }
    // And a program built without the switch says nothing at all.
    let plain = Command::new(BIN)
        .current_dir(&dir)
        .arg("build")
        .arg(root().join("tests/audit/cycle.keal"))
        .output()
        .expect("cannot run keal build");
    assert!(plain.status.success());
    let quiet = Command::new(dir.join("cycle")).output().expect("cannot run the binary");
    assert!(
        !String::from_utf8_lossy(&quiet.stderr).contains("audit:"),
        "an unaudited build audited anyway"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The site in the repository is what its generator would write today.
///
/// The pages are generated from `docs/*.md` and committed, so an edit to a
/// document that never reaches the site leaves a page saying something the
/// repository no longer says — a drift nothing would surface until somebody
/// happened to regenerate. This regenerates into a copy of `site/` and
/// compares, so the tree is never written to. Skipped without Python, and
/// without the binary the standard-library page is built from.
#[test]
fn the_site_is_what_its_generator_would_write() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipping: no `python3`");
        return;
    }
    let dir = std::env::temp_dir().join("keal-site-drift");
    let _ = std::fs::remove_dir_all(&dir);
    let site = dir.join("site");
    std::fs::create_dir_all(&site).expect("cannot make a site directory");
    // The generator writes beside itself, so it is copied somewhere else
    // along with everything it reads that lives under `site/`.
    for entry in std::fs::read_dir(root().join("site")).expect("cannot read site/") {
        let path = entry.expect("cannot read a site entry").path();
        if path.is_file() {
            let name = path.file_name().expect("a file has a name");
            std::fs::copy(&path, site.join(name)).expect("cannot copy a site file");
        }
    }
    // `ROOT` is the generator's parent, so the documents it converts have to
    // be reachable from there. Copied rather than linked: a symlink on
    // Windows wants Developer Mode or elevation, and this is a few hundred
    // kilobytes — a test that skips on a platform is a test that platform
    // does not have.
    for name in ["docs", "README.md", "TUTORIAL.md", "CONTRIBUTING.md"] {
        copy_into(&root().join(name), &dir.join(name));
    }
    let built = Command::new("python3")
        .arg(site.join("build.py"))
        .arg(root().join("target/release/keal"))
        .output()
        .expect("cannot run the site generator");
    assert!(
        built.status.success(),
        "the site generator failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    // Two ways a page can differ, and they want different sentences: one is
    // a page saying something else, the other is a generator writing `\r\n`.
    // Listing twenty pages that differ only in line endings buries the one
    // that says something else, which is how a mojibake page went unread.
    let mut changed = Vec::new();
    let mut only_endings = Vec::new();
    for entry in std::fs::read_dir(&site).expect("cannot read the rebuilt site") {
        let path = entry.expect("cannot read an entry").path();
        if !path.extension().map(|e| e == "html").unwrap_or(false) {
            continue;
        }
        let name = path.file_name().expect("a file has a name").to_string_lossy().into_owned();
        let built = std::fs::read(&path).unwrap_or_default();
        let committed = std::fs::read(root().join("site").join(&name)).unwrap_or_default();
        if built == committed {
            continue;
        }
        let flatten = |b: &[u8]| -> Vec<u8> {
            let mut out = Vec::with_capacity(b.len());
            for c in b {
                if *c != b'\r' {
                    out.push(*c);
                }
            }
            out
        };
        if flatten(&built) == flatten(&committed) {
            only_endings.push(name);
        } else {
            changed.push(name);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        changed.is_empty(),
        "these pages say something the generator would not write; run `python3 site/build.py`: {}",
        changed.join(", ")
    );
    assert!(
        only_endings.is_empty(),
        "the generator wrote {} page(s) with different line endings, which it must not: {}",
        only_endings.len(),
        only_endings.join(", ")
    );
}

/// The language server, driven the way an editor drives it: framed
/// JSON-RPC over a pipe.
///
/// What is checked is that it answers — an editor that waits forever on a
/// request is worse than one that gets `null` — and that the answers are
/// about the buffer rather than the file, which is the whole reason the
/// loader grew an overlay.
#[test]
fn the_language_server_answers() {
    use std::io::{Read, Write};

    let dir = std::env::temp_dir().join("keal-lsp-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a directory");
    let file = dir.join("main.keal");
    let on_disk = "enum Level { Debug, Info }\nval here = Level.Debug\nprintln(here)\n";
    std::fs::write(&file, on_disk).unwrap();

    // What the editor is holding differs from what is on disk: a type error
    // on a line the file does not have. Only an overlay can see it.
    let buffer = "enum Level { Debug, Info }\nval here = Level.Debug\nprintln(here)\nval bad: Int = \"x\"\n";
    // The URI an editor would actually send: forward slashes throughout,
    // and a leading one before a Windows drive letter. Building it from a
    // `Display`ed path instead is how this test failed on Windows and
    // nowhere else — `file://D:\a\...` puts `\a` inside a JSON string,
    // which is not an escape, so every message carrying a URI was thrown
    // away unparsed while the two that carried none went through.
    let uri = {
        let text = file.to_string_lossy().replace('\\', "/");
        if text.starts_with('/') {
            format!("file://{}", text)
        } else {
            format!("file:///{}", text)
        }
    };

    let frame = |v: &str| format!("Content-Length: {}\r\n\r\n{}", v.len(), v);
    let mut input = String::new();
    input.push_str(&frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#));
    input.push_str(&frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{}","text":"{}"}}}}}}"#,
        uri,
        buffer.replace('"', "\\\"").replace('\n', "\\n")
    )));
    // Hover on `here` in `val here = ...`, line 1, character 4.
    input.push_str(&frame(&format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{{"textDocument":{{"uri":"{}"}},"position":{{"line":1,"character":4}}}}}}"#,
        uri
    )));
    // Definition of `here` from its use on line 2.
    input.push_str(&frame(&format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"textDocument/definition","params":{{"textDocument":{{"uri":"{}"}},"position":{{"line":2,"character":9}}}}}}"#,
        uri
    )));
    input.push_str(&frame(&format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"textDocument/documentSymbol","params":{{"textDocument":{{"uri":"{}"}}}}}}"#,
        uri
    )));
    input.push_str(&frame(r#"{"jsonrpc":"2.0","id":5,"method":"shutdown","params":{}}"#));
    input.push_str(&frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#));

    let mut child = Command::new(BIN)
        .arg("lsp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("cannot start the language server");
    child.stdin.as_mut().unwrap().write_all(input.as_bytes()).unwrap();
    drop(child.stdin.take());
    let mut out = String::new();
    child.stdout.as_mut().unwrap().read_to_string(&mut out).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "the language server did not exit cleanly");

    assert!(out.contains("Content-Length:"), "nothing was framed:\n{}", out);
    assert!(out.contains("\"hoverProvider\":true"), "it did not offer hover:\n{}", out);
    // The diagnostic is on line 3, which exists only in the buffer.
    assert!(
        out.contains("publishDiagnostics"),
        "no diagnostics were published:\n{}",
        out
    );
    assert!(
        out.contains("but `Int` was expected"),
        "the unsaved buffer was not what it checked:\n{}",
        out
    );
    assert!(
        out.contains("here: Level"),
        "hover did not name the type:\n{}",
        out
    );
    assert!(
        out.contains("documentSymbol") || out.contains("\"Level\""),
        "the outline is missing:\n{}",
        out
    );
    // `here` is deliberately a name the prelude also binds, inside
    // `walkDir`. Names are not unique across a program, and resolving one
    // by taking the first match in the declaration list answers with the
    // prelude's — so hovering this ordinary local reported a type from a
    // file the program never opened, and going to its definition offered a
    // pseudo-file. Both must land in the buffer the cursor is in.
    assert!(
        !out.contains("%3Cprelude%3E"),
        "a name the prelude also binds resolved to the prelude:\n{}",
        out
    );
    assert!(
        out.contains("main.keal\"}") || out.contains("main.keal\","),
        "the definition did not land in the open file:\n{}",
        out
    );

    // A URI has an authority before its first slash and it is dropped —
    // except that a Windows drive letter looks exactly like one. Dropping
    // `C:` from `file://C:/x` answers `/x`: a plausible path pointing
    // somewhere else, so the buffer would be keyed on a file nothing ever
    // asks about and the client would get silence, which is the same
    // symptom as a crash and harder to find.
    //
    // No file is written for this. The overlay is what the server reads,
    // so a path that exists nowhere still opens, checks and reports — which
    // is what lets a Windows-shaped URI be tested on any machine.
    for (spelling, must_contain) in [
        ("file://C:/nowhere/main.keal", "C:"),
        // And the half that must keep working: a real authority is still
        // dropped, so the fix above cannot have become "never strip".
        ("file://localhost/nowhere/main.keal", "/nowhere/main.keal"),
    ] {
        let mut probe = String::new();
        probe.push_str(&frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#));
        probe.push_str(&frame(&format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{}","text":"{}"}}}}}}"#,
            spelling,
            buffer.replace('"', "\\\"").replace('\n', "\\n")
        )));
        probe.push_str(&frame(&format!(
            r#"{{"jsonrpc":"2.0","id":9,"method":"textDocument/hover","params":{{"textDocument":{{"uri":"{}"}},"position":{{"line":1,"character":4}}}}}}"#,
            spelling
        )));
        probe.push_str(&frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#));

        let mut child = Command::new(BIN)
            .arg("lsp")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("cannot start the language server");
        child.stdin.as_mut().unwrap().write_all(probe.as_bytes()).unwrap();
        drop(child.stdin.take());
        let mut out = String::new();
        child.stdout.as_mut().unwrap().read_to_string(&mut out).unwrap();
        child.wait().unwrap();
        assert!(
            out.contains(must_contain),
            "`{}` did not resolve to a path containing `{}`:\n{}",
            spelling,
            must_contain,
            out
        );
        assert!(
            out.contains("here: Level"),
            "`{}` opened a document the server then could not answer about:\n{}",
            spelling,
            out
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Two spellings, one file — and the buffer has to win anyway.
///
/// macOS and Windows open `lib.keal` when the file on disk is `Lib.keal`;
/// `PathBuf` compares those two as different. So an editor holding
/// `Lib.keal` and an `import "./lib.keal"` used to miss each other in the
/// overlay, and the checker answered from the copy on disk without saying
/// it had: diagnostics one save behind, silently. The overlay is keyed by
/// what the filesystem calls the file now, which is the only test that can
/// agree with the filesystem.
///
/// Whether the two spellings *are* one file is the filesystem's answer, not
/// this test's, so it asks before it asserts — and both answers are worth
/// pinning. Where they are one file the buffer must win; where they are
/// two, the import must fail to read rather than find something.
#[test]
fn an_unsaved_buffer_wins_however_its_path_is_spelled() {
    use std::io::{Read, Write};

    let dir = std::env::temp_dir().join("keal-lsp-case");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a directory");
    let lib = dir.join("Lib.keal");
    std::fs::write(&lib, "public func hello(): Int { return 1 }\n").unwrap();
    let main = dir.join("main.keal");
    std::fs::write(&main, "import \"./lib.keal\"\nval x: Int = hello()\nprintln(x)\n").unwrap();

    // Does this filesystem fold case? Ask it rather than guess from the
    // target triple: macOS can be case-sensitive and Linux can be mounted
    // case-insensitive.
    let folds_case = std::fs::read_to_string(dir.join("lib.keal")).is_ok();

    let uri = |p: &std::path::Path| {
        let text = p.to_string_lossy().replace('\\', "/");
        if text.starts_with('/') { format!("file://{}", text) } else { format!("file:///{}", text) }
    };
    let frame = |v: &str| format!("Content-Length: {}\r\n\r\n{}", v.len(), v);
    let open = |p: &std::path::Path, text: &str| {
        frame(&format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{}","text":"{}"}}}}}}"#,
            uri(p),
            text.replace('"', "\\\"").replace('\n', "\\n")
        ))
    };

    let mut input = String::new();
    input.push_str(&frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#));
    // The editor holds the library under the name the disk uses, with a
    // change that has not been saved: `hello` gives a `String` now.
    input.push_str(&open(&lib, "public func hello(): String { return \"one\" }\n"));
    input.push_str(&open(&main, "import \"./lib.keal\"\nval x: Int = hello()\nprintln(x)\n"));
    input.push_str(&frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#));

    let mut child = Command::new(BIN)
        .arg("lsp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("cannot start the language server");
    child.stdin.as_mut().unwrap().write_all(input.as_bytes()).unwrap();
    drop(child.stdin.take());
    let mut out = String::new();
    child.stdout.as_mut().unwrap().read_to_string(&mut out).unwrap();
    child.wait().unwrap();

    if folds_case {
        assert!(
            out.contains("but `Int` was expected") || out.contains("has type `String`"),
            "the import read the file on disk instead of the buffer the editor is holding:\n{}",
            out
        );
    } else {
        assert!(
            out.contains("cannot read"),
            "`lib.keal` is a different file here, so the import should have failed:\n{}",
            out
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The index: a git repository holding one small file per package, saying
/// where that package lives and nothing else. The whole chain in one test —
/// find a package by a word in its description, write it into the manifest
/// pinned to an exact tag, fetch it, import it, run it — because every step
/// of it is only worth anything if the next one works.
#[test]
fn the_index_finds_a_package_and_pins_it() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: no `git`");
        return;
    }
    let dir = std::env::temp_dir().join("keal-index-test");
    let _ = std::fs::remove_dir_all(&dir);
    let (pkg, index, home, app) =
        (dir.join("geometry"), dir.join("index"), dir.join("home"), dir.join("app"));
    for d in [&pkg, &index.join("packages"), &home, &app] {
        std::fs::create_dir_all(d).expect("cannot make a directory");
    }
    let git = |args: &[&str], at: &Path| {
        let out = Command::new("git").args(args).current_dir(at).output().expect("cannot run git");
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    };
    let init = |at: &Path| {
        git(&["init", "-q", "."], at);
        git(&["config", "user.email", "t@example.com"], at);
        git(&["config", "user.name", "Test"], at);
    };

    // A package with three version tags and one that is not a version. The
    // interesting pair is v1.2.0 and v1.10.0: sorted as text the wrong one
    // wins, and everybody reading it expects the other.
    std::fs::write(pkg.join("shapes.keal"), "public func area(): Int { 7 }\n").unwrap();
    init(&pkg);
    git(&["add", "-A"], &pkg);
    git(&["commit", "-qm", "x"], &pkg);
    for tag in ["v1.0.0", "v1.2.0", "v1.10.0", "nightly"] {
        git(&["tag", tag], &pkg);
    }

    std::fs::write(
        index.join("packages").join("geometry.toml"),
        format!(
            "[package]\nname = \"geometry\"\ngit = \"{}\"\ndescription = \"points, lines and the arithmetic between them\"\n",
            pkg.display()
        ),
    )
    .unwrap();
    // A file that is not an entry: one bad contribution must not make the
    // index unreadable for everybody standing behind it.
    std::fs::write(index.join("packages").join("broken.toml"), "not a package at all\n").unwrap();
    init(&index);
    git(&["add", "-A"], &index);
    git(&["commit", "-qm", "x"], &index);

    std::fs::write(app.join("keal.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\n")
        .unwrap();
    let keal = |args: &[&str]| {
        Command::new(BIN)
            .args(args)
            .current_dir(&app)
            .env("KEAL_INDEX", &index)
            .env("KEAL_HOME", &home)
            .output()
            .expect("cannot run keal")
    };

    // Found by a word in its description, not by its name.
    let out = keal(&["search", "arithmetic"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "search failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("geometry"), "the index did not find it:\n{}", text);

    // No tag named, so the newest is chosen — numerically, and ignoring
    // what is not a version — and written down as an exact pin.
    let out = keal(&["add", "geometry"]);
    assert!(out.status.success(), "add failed: {}", String::from_utf8_lossy(&out.stderr));
    let manifest = std::fs::read_to_string(app.join("keal.toml")).unwrap();
    assert!(
        manifest.contains("tag = \"v1.10.0\""),
        "the newest tag was not the one written:\n{}",
        manifest
    );
    assert!(manifest.contains("name = \"app\""), "the rest of the manifest was lost");

    // A pin is a decision, so a second `add` of the same name refuses.
    let out = keal(&["add", "geometry"]);
    assert!(!out.status.success(), "adding the same dependency twice was allowed");

    // A tag nobody published is refused where it is typed, and the message
    // says what there is instead.
    let out = keal(&["add", "geometry@v9.9.9"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "a tag that does not exist was written");
    assert!(err.contains("v1.10.0"), "the refusal did not name the real tags:\n{}", err);

    // And the point of all of it: the package is on disk and the program
    // that imports it runs.
    let out = keal(&["fetch"]);
    assert!(out.status.success(), "fetch failed: {}", String::from_utf8_lossy(&out.stderr));
    std::fs::write(
        app.join("main.keal"),
        "import \"dep:geometry/shapes.keal\"\nprintln(area())\n",
    )
    .unwrap();
    let out = keal(&["run", "main.keal"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "7",
        "the added package did not run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Dependencies that have dependencies: an app that names one library,
/// which names another. Both land in the app's own `.keal/deps`, and the
/// library's own `dep:` import reaches that copy rather than looking inside
/// itself — which is the whole reason a `dep:` resolves against the
/// outermost manifest. Two versions of one name is refused, by name, with
/// both askers.
#[test]
fn a_dependency_may_have_dependencies() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: no `git`");
        return;
    }
    let dir = std::env::temp_dir().join("keal-transitive-test");
    let _ = std::fs::remove_dir_all(&dir);
    let (deep, mid, app) = (dir.join("deep"), dir.join("mid"), dir.join("app"));
    for d in [&deep, &mid, &app] {
        std::fs::create_dir_all(d).expect("cannot make a directory");
    }
    let git = |args: &[&str], at: &Path| {
        let out = Command::new("git").args(args).current_dir(at).output().expect("cannot run git");
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    };
    let commit = |at: &Path, tag: &str| {
        git(&["init", "-q", "."], at);
        git(&["config", "user.email", "t@example.com"], at);
        git(&["config", "user.name", "Test"], at);
        git(&["add", "-A"], at);
        git(&["commit", "-qm", "x"], at);
        git(&["tag", tag], at);
    };

    std::fs::write(deep.join("deep.keal"), "public func deep(): Int { 42 }\n").unwrap();
    commit(&deep, "v1");

    std::fs::write(
        mid.join("keal.toml"),
        format!(
            "[package]\nname = \"mid\"\nversion = \"0.1.0\"\n\n[dependencies]\ndeep = {{ git = \"{}\", tag = \"v1\" }}\n",
            deep.display()
        ),
    )
    .unwrap();
    std::fs::write(
        mid.join("mid.keal"),
        "import \"dep:deep/deep.keal\"\npublic func middle(): Int { deep() + 1 }\n",
    )
    .unwrap();
    commit(&mid, "v1");

    std::fs::write(
        app.join("keal.toml"),
        format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmid = {{ git = \"{}\", tag = \"v1\" }}\n",
            mid.display()
        ),
    )
    .unwrap();
    std::fs::write(
        app.join("main.keal"),
        "import \"dep:mid/mid.keal\"\nassert(middle() == 43, \"through two dependencies\")\n",
    )
    .unwrap();

    let fetched = Command::new(BIN).arg("fetch").current_dir(&app).output().expect("cannot fetch");
    assert!(fetched.status.success(), "fetch failed:\n{}", String::from_utf8_lossy(&fetched.stderr));
    assert!(app.join(".keal/deps/deep/deep.keal").exists(), "the deep dependency was not fetched flat");
    assert!(app.join("keal.lock").exists(), "no lockfile");
    let lock = std::fs::read_to_string(app.join("keal.lock")).unwrap();
    assert!(lock.contains("asked_by = \"mid\""), "the lockfile does not say who asked:\n{}", lock);

    for engine in ENGINES {
        let ran = Command::new(BIN)
            .args([engine, "main.keal"])
            .current_dir(&app)
            .output()
            .expect("cannot run the program");
        assert!(
            ran.status.success(),
            "the transitive dependency did not run on {}:\n{}",
            engine,
            String::from_utf8_lossy(&ran.stderr)
        );
    }

    // And two versions of one name, which nothing can pick between.
    std::fs::write(deep.join("deep.keal"), "public func deep(): Int { 99 }\n").unwrap();
    git(&["add", "-A"], &deep);
    git(&["commit", "-qm", "y"], &deep);
    git(&["tag", "v2"], &deep);
    let manifest = std::fs::read_to_string(app.join("keal.toml")).unwrap();
    std::fs::write(
        app.join("keal.toml"),
        format!("{}deep = {{ git = \"{}\", tag = \"v2\" }}\n", manifest, deep.display()),
    )
    .unwrap();
    let clash = Command::new(BIN).arg("fetch").current_dir(&app).output().expect("cannot fetch");
    assert!(!clash.status.success(), "a version clash was accepted");
    let said = String::from_utf8_lossy(&clash.stderr);
    assert!(
        said.contains("two versions of `deep`") && said.contains("app wants") && said.contains("mid wants"),
        "the clash did not name both askers:\n{}",
        said
    );
    let _ = std::fs::remove_dir_all(&dir);
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

/// A program that prints and then fails must tell the two things in the order
/// it did them, on every engine.
///
/// This is only visible where the two streams MEET — a terminal, a log, a
/// `2>&1` — and every other comparison in this file reads them apart. So the
/// native backend disagreed here from the day it was written and nothing
/// could see it: C leaves stdout fully buffered when it is not a terminal, so
/// the failure reached a pipe ahead of the output that came before it, while
/// the interpreters' writer flushes each line. Same words, same exit status,
/// the story backwards.
#[test]
fn output_and_failure_arrive_in_the_order_they_happened() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    let src = "tests/runtime/print_then_fail.keal";
    let dir = std::env::temp_dir().join("keal-stream-order");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a build directory");
    let exe = dir.join("print_then_fail");
    let built = Command::new(BIN)
        .args(["build", src])
        .arg("-o")
        .arg(&exe)
        .current_dir(root())
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the fixture did not compile:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    // Both streams into ONE file, which is the only arrangement that records
    // an order at all.
    let merged = |mut cmd: Command, tag: &str| -> String {
        let path = dir.join(tag);
        let f = std::fs::File::create(&path).expect("cannot make a capture file");
        let g = f.try_clone().expect("cannot share the capture file");
        cmd.stdout(std::process::Stdio::from(f));
        cmd.stderr(std::process::Stdio::from(g));
        cmd.current_dir(root()).status().expect("cannot run the program");
        std::fs::read_to_string(&path).expect("cannot read the capture")
    };

    let mut runs = vec![("native", merged(Command::new(&exe), "native"))];
    for engine in ENGINES {
        let mut cmd = Command::new(BIN);
        cmd.args([engine, src]);
        runs.push((engine, merged(cmd, engine.trim_start_matches('-'))));
    }

    for (name, text) in &runs {
        let printed = text
            .find("before the failure")
            .unwrap_or_else(|| panic!("{}: the program's own output is missing:\n{}", name, text));
        let failed = text
            .find("runtime error")
            .unwrap_or_else(|| panic!("{}: the failure is missing:\n{}", name, text));
        assert!(
            printed < failed,
            "{} tells the failure before the output that preceded it:\n{}",
            name,
            text
        );
    }
}

/// A closed pipe is an ERROR on every engine, not a death on one of them.
///
/// `prog | head` closes the pipe after ten lines. Unix tradition says the
/// next write raises SIGPIPE and the kernel ends the process — 141, nothing
/// on stderr — and that is what the compiled program used to do, while the
/// interpreters reported "cannot write to standard output: Broken pipe" and
/// exited 1. Rust's runtime ignores SIGPIPE and this backend did not.
///
/// The tree-walker is the specification, so the backend follows it. This
/// pins that they end the same way, since the difference is in the exit
/// status and in stderr, and the corpus comparison reads neither.
#[test]
fn a_closed_pipe_ends_every_engine_the_same_way() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    let dir = std::env::temp_dir().join("keal-broken-pipe");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a build directory");
    let src = dir.join("flood.keal");
    std::fs::write(&src, "var i = 0\nwhile (i < 200000) { println(\"line ${i}\") i += 1 }\n")
        .expect("cannot write the fixture");
    let exe = dir.join("flood");
    let built = Command::new(BIN)
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the fixture did not compile:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    // Read a little, then drop the pipe — which is what `head` does.
    let run = |mut cmd: Command| -> (Option<i32>, String) {
        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("cannot start the program");
        {
            use std::io::Read;
            let mut out = child.stdout.take().expect("no stdout");
            let mut head = [0u8; 64];
            let _ = out.read(&mut head);
        }
        let done = child.wait_with_output().expect("cannot wait");
        (done.status.code(), String::from_utf8_lossy(&done.stderr).into_owned())
    };

    let (native_code, native_err) = run(Command::new(&exe));
    for engine in ENGINES {
        let mut cmd = Command::new(BIN);
        cmd.args([engine, src.to_str().unwrap()]);
        let (code, err) = run(cmd);
        assert_eq!(
            native_code, code,
            "{} ends at {:?} where the compiled program ends at {:?}",
            engine, code, native_code
        );
        let line = |s: &str| s.lines().next().unwrap_or("").to_string();
        assert_eq!(
            line(&native_err),
            line(&err),
            "{} and the compiled program say different things about it",
            engine
        );
    }
    assert_eq!(native_code, Some(1), "a closed pipe is an error, not a signal");
    assert!(
        native_err.contains("cannot write to standard output"),
        "and it says so: {:?}",
        native_err
    );
}

/// Actors printing at once must not tear a line.
///
/// `fwrite` and `fputc` each lock the stream and do not lock together, so the
/// backend's `println` — a write then a newline — let another thread's line
/// land between a line and its own newline: two lines joined, then an empty
/// one. The interpreters emit a line under one lock and cannot do it. It is a
/// disagreement about OUTPUT, so every corpus comparison should have caught
/// it, and none did: no program in the corpus prints from more than one
/// thread.
///
/// The SIZE of the fixture is the test. Two actors and 800 lines caught this
/// on every run of one machine and 9% of runs on another — a green that means
/// nothing and a red that gets called flaky. Eight actors and 8,000 lines tear
/// about 250 of them per run here, 20 runs out of 20, which leaves room for a
/// scheduler an order of magnitude less obliging — measured at four runs in
/// five there, against every run here on the same fixture, which is how much
/// this quantity varies between machines. Longer lines were tried first and
/// made it WORSE: what matters is how many times the window opens, not how
/// wide it is.
///
/// The fixture is built here rather than kept in `tests/`, because its output
/// is 8,000 lines in an order that is nobody's business and every directory
/// there belongs to a harness that would compare it.
#[test]
fn concurrent_actors_do_not_tear_a_line() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    const ACTORS: &str = "abcdefgh";
    const EACH: usize = 1000;
    const WIDTH: usize = 36;

    let mut src = String::from("record M(val tag: String)\nval sys: ActorSystem<M> = ActorSystem()\n");
    for a in ACTORS.chars() {
        src.push_str(&format!(
            "val {a} = sys.spawn({{ self, m -> var i = 0\n    while (i < {EACH}) {{ println(\"{a}-{body}\") i++ }} }})\n",
            a = a,
            EACH = EACH,
            body = a.to_ascii_uppercase().to_string().repeat(WIDTH),
        ));
    }
    for a in ACTORS.chars() {
        src.push_str(&format!("{a}.send(M(\"{a}\"))\n", a = a));
    }
    src.push_str("sys.run()\n");

    let dir = std::env::temp_dir().join("keal-torn-lines");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a build directory");
    let path = dir.join("torn.keal");
    std::fs::write(&path, &src).expect("cannot write the fixture");
    let exe = dir.join("torn");
    let built = Command::new(BIN)
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the fixture did not compile:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let expected = ACTORS.len() * EACH;
    // Tearing is a race, so one run proves nothing either way — and the odds
    // are the machine's, not the test's: the defect shows in every run here
    // and four runs in five on Linux aarch64. Five draws rather than three,
    // because the lever for a frequency is the number of draws and three left
    // roughly one miss in 125 on that machine — not a test that fails, a test
    // that looks flaky on some Tuesday and gets disabled.
    //
    // What is MEASURED, on the machine with the worse odds: 40 batches of
    // five, 40 detections of the defect, and 40 batches with the fix in place
    // and no false alarm. What is DERIVED from a per-run 0.8: a miss in three
    // thousand. Forty batches cannot tell that apart from a miss in a hundred
    // — they bound the miss rate at 7% or better — and measuring the derived
    // figure would take thousands of batches and is not worth the machine
    // time. The bound is what this test stands on.
    for attempt in 1..=5 {
        let out = Command::new(&exe).output().expect("cannot run the fixture");
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), expected, "attempt {}: wrong number of lines", attempt);
        for (i, line) in lines.iter().enumerate() {
            let b = line.as_bytes();
            let whole = b.len() == WIDTH + 2
                && ACTORS.contains(b[0] as char)
                && b[1] == b'-'
                && b[2..].iter().all(|&c| c == b[0].to_ascii_uppercase());
            assert!(
                whole,
                "attempt {}: line {} was torn by another thread: {:?}",
                attempt,
                i + 1,
                line
            );
        }
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

/// One fault per `-Werror` name the check below relies on: the smallest C
/// that commits exactly that mistake. Their only job is to be rejected — a
/// flag that rejects nothing lets everything through.
const FAULTS: [(&str, &str); 9] = [
    ("comment", "/* a /* b */\nint main(void){return 0;}\n"),
    // An assignment used as a condition, which GCC and clang both reject
    // under this name. The doubled-equality form `if ((a==b))` was the
    // original probe and was a compiler assumption: it is clang's
    // `-Wparentheses-equality`, and GCC accepts the name, says nothing, and
    // exits 0 — so on GCC the check proved a flag that was not looking at
    // anything. The two compilers put opposite mistakes under one flag name;
    // this is the one they agree on.
    ("parentheses", "int main(void){int a=1,b=2; if (a=b) return 1; return 0;}\n"),
    (
        "incompatible-pointer-types",
        "void f(char* p); int main(void){ f((int*)0); return 0; }\n",
    ),
    ("implicit-function-declaration", "int main(void){ return nowhere_declared(); }\n"),
    ("int-conversion", "int main(void){ char* p = 5; (void)p; return 0; }\n"),
    // The five below are what an open-addressed hash table with function
    // pointers gets wrong, and the barrier was not asking about any of them
    // until it held one. A map's index is `uint64_t` and its entries are
    // `int64_t`, its probe masks a hash, and its key hash is reached through
    // a pointer whose signature the caller casts — every line of that is one
    // of these mistakes waiting to be made.
    //
    // Named by the Linux aarch64 bench, which wrote a fault for each and
    // confirmed GCC 15.2 rejects it. It also tried `strict-aliasing`, found
    // it silent even in a real compile, and said so rather than adding a
    // name that would have proved nothing.
    // `cast-function-type` is deliberately absent, and the reason is the
    // point of the list.
    //
    // It bites — a fault written for it is rejected. But it also rejects
    // every closure this language makes: a `KealClosure` stores its code as
    // `KealCode`, which is `void (*)(void)`, and the call site casts it back
    // to the signature the static type promises. Converting a function
    // pointer to another function pointer type and back is defined in C;
    // only *calling* through the wrong one is not, and nothing here does.
    // Under clang the name pulls in `-Wcast-function-type-strict`, which
    // objects to the idiom itself.
    //
    // So a flag that bites is necessary and not sufficient: it must also not
    // refuse code that is right. Calibration proves the first, and only
    // running it over the corpus proves the second. This one was proposed
    // from a fault, added, and taken back out by the corpus within the hour
    // — which is the barrier working, not failing.
    // From a variable, not a constant: GCC rejects `(char*)1234567` under
    // this name and clang does not — clang's rule is a cast from a SMALLER
    // integer type, which a literal is not. A fault calibrated on one
    // compiler is a compiler assumption, whichever compiler it was, and the
    // pair caught this one in both directions inside an hour.
    (
        "int-to-pointer-cast",
        "int main(void){ int n = 5; char* p = (char*)n; (void)p; return 0; }\n",
    ),
    ("pointer-to-int-cast", "int main(void){ char c; int n = (int)&c; return n & 0; }\n"),
    (
        "sign-compare",
        "int main(void){ int i = -1; unsigned u = 1u; return i < u ? 1 : 0; }\n",
    ),
    ("shift-count-overflow", "int main(void){ int x = 1; return x << 64; }\n"),
];

/// The generated C must compile *quietly*, not merely compile.
///
/// A warning is what the C compiler says when it can build the file and
/// suspects the file is not what was meant, and the two this pins down were
/// both of that kind: a string literal holding `/*` opened a comment inside
/// the comment it is echoed into, and a comparison handed to a short-circuit
/// branch arrived wrapped in the second pair of parentheses that C reads as
/// "this assignment is deliberate". Neither stopped the build; both printed
/// on every bootstrap, which is how a real warning would have gone unread.
///
/// `-Werror=` on the three names, rather than `-Wall`: this asks the compiler
/// about the things that went wrong and does not make the suite hostage to
/// every opinion a future version of it acquires.
///
/// Two of them are cosmetic. The other three — `incompatible-pointer-types`,
/// `implicit-function-declaration`, `int-conversion` — each mean the backend
/// emitted code the C compiler can see is wrong, which is a mis-compilation
/// whatever `cc` decides to do about it. One shipped: a named `func` used as
/// a value became a bare function pointer where a counted closure was
/// expected, and the program took a bus error on the first call. The
/// compiler had said so, in one line, on every build.
///
/// The other two names come from the two consumer sessions, who hit exactly
/// those classes — "call to undeclared function `K_App_m_build`" and
/// "incompatible integer to pointer conversion" — and built the same barrier
/// on their side before suggesting it here.
///
/// What each name covers is not identical across compilers, and `parentheses`
/// is the one where it matters: clang's catches the doubled `if ((a == b))`
/// this emitter twice produced, and GCC's does not catch it at all. So the
/// flag is a second net, finer on clang than on GCC, and the thing that
/// actually guarantees the emitter no longer writes that shape is `open_if` —
/// one place, structural, the same on every platform. A barrier whose reach
/// varies by toolchain is worth having and worth not relying on.
#[test]
fn the_generated_c_compiles_without_warnings() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }

    let dir = std::env::temp_dir().join("keal-c-warnings");
    std::fs::create_dir_all(&dir).expect("cannot make a build directory");
    let csrc = dir.join("out.c");

    // First, that the instrument works.
    //
    // Everything below asserts an ABSENCE — that no warning came back — and
    // a check of that shape passes the moment the thing doing the checking
    // stops working. `cc` accepts an unknown `-Werror=` name with a warning
    // and exit 0, so a misspelling, a dropped flag, or a compiler that never
    // had one of these would leave this test green while it verified
    // nothing. So each name is given a fault of its own and must reject it.
    // A name this compiler does not know is a check that cannot run, and
    // says so, rather than one that quietly passes.
    let mut proven: Vec<&str> = Vec::new();
    for (name, fault) in FAULTS {
        let fsrc = dir.join(format!("fault-{}.c", name));
        std::fs::write(&fsrc, fault).expect("cannot write the fault");
        // Compiled exactly the way the corpus below is compiled, and that
        // sameness is the whole safety property — not the mode itself.
        //
        // Some warnings come only from the optimisation passes: on GCC 15.2,
        // `-Werror=uninitialized` and `-Werror=return-type` say nothing under
        // `-fsyntax-only` and speak under `-c -O1`. Both of us read that as
        // "such a name would be skipped in silence", and both of us were
        // wrong. The skip fires on `unknown warning option`, which a compiler
        // prints for a name it does not have; a name it HAS and cannot reach
        // under these flags exits 0 saying nothing, reaches the assertion,
        // and fails loudly with "did not reject the fault written for it".
        //
        // So the mode is free to be the cheap one. The nine names in the
        // table were measured to bite identically either way, and a real
        // compile of the corpus costs seven times more on Windows than on
        // macOS — for nothing that is being asked today. The day someone
        // adds a name that needs the optimiser, this loop says so in one
        // sentence and both sides move together.
        let out = Command::new(&cc)
            .args(["-std=c11", "-fsyntax-only", &format!("-Werror={}", name)])
            .arg(&fsrc)
            .output()
            .expect("cannot run the C compiler");
        let said = String::from_utf8_lossy(&out.stderr).into_owned();
        if said.contains("unknown warning option") || said.contains("no option") {
            // Out loud, and without naming a cause it cannot know. Two
            // things produce this and they are indistinguishable from here:
            // a compiler that never had the name, and a name misspelled in
            // the table above. Guessing the first would send a reader to
            // check their toolchain over a typo three lines away.
            println!(
                "skipping -Werror={name}: `{cc}` says it has no such warning. \
                 Either this compiler does not have it — then nothing is \
                 wrong and one check fewer runs — or the name is misspelled \
                 in FAULTS, which the compiler cannot tell apart. Its words: \
                 {said}",
                name = name,
                cc = cc,
                said = said.trim()
            );
            continue;
        }
        // The message has to stand on its own, because the interesting case
        // is the one where the compiler said NOTHING — a flag that stopped
        // biting produces no diagnostic, so a report that only echoes the
        // compiler's output ends in a colon and nothing after it. What a
        // reader needs is the C that was supposed to be rejected and what
        // came back instead.
        assert!(
            !out.status.success() && said.contains(name),
            "-Werror={name} did not reject the fault written for it, so the \
             absence asserted below would mean nothing.\n\
             \x20 the fault, which this flag exists to catch:\n{fault}\
             \x20 `{cc}` exited {code} and said: {said}\n\
             \x20 fix the fault so it commits that mistake again, or drop \
             the name if the compiler has stopped having it.",
            name = name,
            fault = fault,
            cc = cc,
            code = out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "on a signal".into()),
            said = if said.trim().is_empty() { "nothing at all".to_string() } else { format!("\n{}", said) }
        );
        proven.push(name);
    }
    assert!(
        !proven.is_empty(),
        "no `-Werror` name this compiler has was proven to bite, so the rest \
         of this test asserts an absence nothing is looking for"
    );

    // The flags the corpus is compiled under are exactly the ones just
    // proven, and not a second list that could drift from this one — and it
    // is compiled the same way they were proven, which is what makes the
    // calibration a promise about this run rather than about some other one.
    let mut flags: Vec<String> =
        vec!["-std=c11".to_string(), "-fsyntax-only".to_string()];
    flags.extend(proven.iter().map(|n| format!("-Werror={}", n)));

    // Every program the native corpus has, not just the one written for
    // this: a warning names a shape, and the shape can arrive from anywhere.
    for file in keal_files("tests/native") {
        let path = relative(&file);
        let emitted = keal(&["emit-c", &path]);
        assert!(emitted.success, "{} did not emit C:\n{}", path, emitted.stderr);
        std::fs::write(&csrc, &emitted.stdout).expect("cannot write the generated C");

        let built = Command::new(&cc)
            .args(&flags)
            .arg(&csrc)
            .output()
            .expect("cannot run the C compiler");
        // The step this whole test exists for, so its report has to carry
        // the program that failed and what came back. A compiler that
        // refuses and says nothing is a second thing wrong on top of the
        // first, and the report says which one it is looking at.
        let complaint = String::from_utf8_lossy(&built.stderr).into_owned();
        assert!(
            built.status.success(),
            "the C generated for {path} does not compile cleanly under \
             {flags}.\n  `{cc}` said: {said}",
            path = path,
            flags = flags[2..].join(" "),
            cc = cc,
            said = if complaint.trim().is_empty() {
                "nothing at all, which is a second thing wrong: it refused \
                 the file without a diagnostic"
                    .to_string()
            } else {
                format!("\n{}", complaint)
            }
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Fifteen lines with a condition variable in them, which the sanitizer must
/// survive before anything it says about a real program can be believed.
///
/// This is the `-Werror` self-check again, one floor down: prove the
/// instrument works, then trust its measurement. On Ubuntu 26.04 aarch64
/// this program dies with SIGILL thirty times out of thirty under
/// ThreadSanitizer — a BTI landing-pad fault in glibc's `__sigsetjmp`,
/// reached from TSan's own `pthread_cond_wait` interceptor. Both libraries
/// are built with branch protection by the distribution, so no flag we pass
/// to `cc` touches either one. It is clean on macOS ARM, so it is the
/// GNU/Linux toolchain and not the ISA.
const TSAN_PROBE: &str = "\
#include <pthread.h>\n\
#include <stdio.h>\n\
static pthread_mutex_t mu = PTHREAD_MUTEX_INITIALIZER;\n\
static pthread_cond_t cv = PTHREAD_COND_INITIALIZER;\n\
static int ready;\n\
static void *worker(void *p){ (void)p;\n\
  pthread_mutex_lock(&mu);\n\
  while(!ready) pthread_cond_wait(&cv, &mu);\n\
  pthread_mutex_unlock(&mu); return 0; }\n\
int main(void){ pthread_t t[6];\n\
  for(int i=0;i<6;i++) pthread_create(&t[i],0,worker,0);\n\
  pthread_mutex_lock(&mu); ready=1; pthread_cond_broadcast(&cv);\n\
  pthread_mutex_unlock(&mu);\n\
  for(int i=0;i<6;i++) pthread_join(t[i],0);\n\
  printf(\"done\\n\"); return 0; }\n";

/// The threaded scheduler under ThreadSanitizer: the mesh program — eight
/// actors fanning echoes at each other while posting into one outbox —
/// builds with `-fsanitize=thread` and must come back clean, five runs in
/// a row. Skipped when no C compiler is installed, when this compiler
/// cannot link the sanitizer runtime, and when the sanitizer's own runtime
/// cannot survive a condition variable — so the suite stays green on
/// machines that cannot run the check rather than pretending they did.
#[test]
fn actors_are_clean_under_thread_sanitizer() {
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
    // Calibrate before measuring. The old guard asked only whether `cc`
    // could LINK the sanitizer, which is not the same question as whether
    // the sanitizer runs — and on a platform where it cannot, the verdict
    // below is about the toolchain while reading like a verdict about Keal.
    {
        let probe_dir = std::env::temp_dir().join("keal-tsan-probe");
        let _ = std::fs::remove_dir_all(&probe_dir);
        std::fs::create_dir_all(&probe_dir).expect("cannot make a build directory");
        let src = probe_dir.join("probe.c");
        let bin = probe_dir.join("probe");
        std::fs::write(&src, TSAN_PROBE).expect("cannot write the probe");
        let built = Command::new(&cc)
            .args(["-O2", "-std=c11", "-pthread", "-fsanitize=thread", "-o"])
            .arg(&bin)
            .arg(&src)
            .output()
            .expect("cannot run the C compiler");
        if built.status.success() {
            // Five runs, because the failure it looks for is not
            // deterministic: one clean run proves nothing about a crash that
            // happens six times in ten.
            for _ in 0..5 {
                let run = Command::new(&bin).output().expect("cannot run the probe");
                if !run.status.success() {
                    println!(
                        "skipping: ThreadSanitizer cannot survive a condition \
                         variable on this machine — fifteen lines of plain \
                         pthread code, no Keal in them, came back {status}. \
                         Whatever it would say about the actor mesh would be \
                         about the toolchain, not about the program.",
                        status = run.status
                    );
                    let _ = std::fs::remove_dir_all(&probe_dir);
                    return;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&probe_dir);
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
        // How it failed, not that it did. A process killed by a signal
        // writes nothing, so echoing its stderr reports a colon and an empty
        // line — and the status, which is the whole story, goes unsaid.
        //
        // `ExitStatus`'s own `Display` is what says it: `signal: 4 (SIGILL)`,
        // the name included, on every platform. Reading `code()` and
        // decoding 128 + n is the shell's convention and not Rust's — on
        // Unix `code()` is `None` for a signal, so an arm matching `c > 128`
        // is dead code that reads like a fix. This test had one.
        assert!(
            out.status.success(),
            "the sanitized binary failed: {status}.\n  it said: {said}",
            status = out.status,
            said = if stderr.trim().is_empty() {
                "nothing at all, so this is not a race the sanitizer found — \
                 the process died before it could report one"
                    .to_string()
            } else {
                format!("\n{}", stderr)
            }
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "total 1117\n",
            "the sanitized binary printed the wrong total"
        );
    }

    // And the same question asked where a worker pool can actually go
    // wrong: two thousand actors and only a handful of threads, so the
    // scan for a free actor and the flag that says one is taken are under
    // real contention. One worker and many are both run — a pool with a
    // race often only shows it at one of the two.
    let many = "tests/native/actor-many.keal";
    let emitted = keal(&["emit-c", many]);
    assert!(emitted.success, "{} did not emit C:\n{}", many, emitted.stderr);
    let csrc = dir.join("many.c");
    let bin = dir.join("many");
    std::fs::write(&csrc, &emitted.stdout).expect("cannot write the generated C");
    let built = Command::new(&cc)
        .args(["-O2", "-std=c11", "-pthread", "-fsanitize=thread", "-o"])
        .arg(&bin)
        .arg(&csrc)
        .arg("-lm")
        .output()
        .expect("cannot run the C compiler");
    assert!(
        built.status.success(),
        "the many-actor program did not build under the sanitizer:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let expected = std::fs::read_to_string(root().join("tests/native/actor-many.expected"))
        .expect("cannot read the expected output");
    for workers in ["1", "2", "16"] {
        let out = Command::new(&bin)
            .env("KEAL_ACTOR_WORKERS", workers)
            .output()
            .expect("cannot run the built binary");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            !stderr.contains("ThreadSanitizer"),
            "the thread sanitizer reported a race with {} worker(s):\n{}",
            workers,
            stderr
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            expected,
            "two thousand actors printed the wrong answer with {} worker(s)",
            workers
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

/// `constexpr` promises the work happens at compile time. The promise it
/// has to keep beyond that is that a compiler asked for the impossible
/// **stops** — a loop that does not end at compile time would be a compiler
/// that does not end, which is worse than a wrong answer. These two live
/// here rather than in the compared corpus because exhausting the budget
/// costs real seconds, and the corpus runs four times over.
#[test]
fn a_constexpr_that_cannot_finish_is_refused() {
    let dir = std::env::temp_dir().join("keal-constexpr-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot make a directory");

    let forever = dir.join("forever.keal");
    std::fs::write(
        &forever,
        "constexpr func spin(): Int {\n    var i = 0\n    while (true) { i += 1 }\n    return i\n}\nconstexpr val X = spin()\n",
    )
    .unwrap();
    let out = Command::new(BIN).arg("check").arg(&forever).output().expect("cannot run keal");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "an endless `constexpr` was accepted");
    assert!(
        err.contains("this `constexpr` did not finish"),
        "the budget did not stop it:\n{}",
        err
    );

    let deep = dir.join("deep.keal");
    std::fs::write(
        &deep,
        "constexpr func down(n: Int): Int { return down(n + 1) }\nconstexpr val X = down(0)\n",
    )
    .unwrap();
    let out = Command::new(BIN).arg("check").arg(&deep).output().expect("cannot run keal");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "bottomless recursion was accepted");
    assert!(
        err.contains("recursed too deep at compile time"),
        "the frame limit did not stop it:\n{}",
        err
    );
}

/// The same question asked of the emitters under `--audit`, which is a
/// different program: counting, the walks over every shape a class, a
/// lambda or a container can hold, and the roots the mark phase starts
/// from. None of that is in the C the test above compares, so for a while
/// the twin's audit was the one thing nothing checked.
#[test]
fn the_emitters_agree_under_the_audit_too() {
    let mut files = keal_files("tests/audit");
    files.extend(keal_files("tests/native"));
    for file in files {
        let path = relative(&file);
        let oracle = keal(&["--audit", "emit-c", &path]);
        let mine = keal(&["--vm", "selfhost/cbackend.keal", "--audit", &path]);
        assert_eq!(
            oracle.stdout, mine.stdout,
            "the audited emitters disagree on {}",
            path
        );
        assert_eq!(
            oracle.status_success(),
            mine.status_success(),
            "the audited emitters disagree on whether {} compiles",
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
    // A JDK is not enough: these build native binaries, so they need the C
    // compiler every other native test asks for. Without this they fail
    // where every one of their neighbours skips.
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
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
        .args(jni_link_args(&jh))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the jbind wrappers did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = jvm_run(dir.join("main"), &jh).output().expect("cannot run the binary");
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
    // A JDK is not enough: these build native binaries, so they need the C
    // compiler every other native test asks for. Without this they fail
    // where every one of their neighbours skips.
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
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
        .args(jni_link_args(&jh))
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
    let ran = jvm_run(dir.join("main"), &jh).output().expect("cannot run the binary");
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
    // A JDK is not enough: these build native binaries, so they need the C
    // compiler every other native test asks for. Without this they fail
    // where every one of their neighbours skips.
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
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
        .args(jni_link_args(&jh))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the program did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = jvm_run(dir.join("main"), &jh).output().expect("cannot run the binary");
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
    // A JDK is not enough: these build native binaries, so they need the C
    // compiler every other native test asks for. Without this they fail
    // where every one of their neighbours skips.
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
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
        .args(jni_link_args(&jh))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the JVM gateway did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = jvm_run(dir.join("localdate"), &jh).output().expect("cannot run the binary");
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
    // A JDK is not enough: these build native binaries, so they need the C
    // compiler every other native test asks for. Without this they fail
    // where every one of their neighbours skips.
    let cc = c_driver();
    if Command::new(&cc).arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler found as `{}`", cc);
        return;
    }
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
        .args(jni_link_args(&jh))
        .output()
        .expect("cannot run keal build");
    assert!(
        built.status.success(),
        "the actor JVM program did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = jvm_run(dir.join("actordate"), &jh).output().expect("cannot run the binary");
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

    // And then something READS it, which the comparison above does not.
    //
    // A snapshot on its own attests that an output has not changed, never
    // that it was ever right: nothing else in this suite consumed this page,
    // so a template that had always been malformed would have a green test
    // and a stable snapshot to go with it. This is the same shape as the
    // file-system defect that CONTRIBUTING rule 8 records, and the remedy is
    // the same one — put a consumer on the other end.
    let doc = &out.stdout;
    let void = ["area", "base", "br", "col", "embed", "hr", "img", "input",
                "link", "meta", "source", "track", "wbr", "!doctype"];
    let mut open: Vec<String> = Vec::new();
    let mut rest = doc.as_str();
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with("!--") {
            continue;
        }
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        if closing {
            let was = open.pop();
            assert_eq!(
                was.as_deref(),
                Some(name.as_str()),
                "the documentation closes `</{}>` where `<{}>` was open",
                name,
                was.as_deref().unwrap_or("nothing")
            );
        } else if !void.contains(&name.as_str()) && !tag.ends_with('/') {
            open.push(name);
        }
    }
    assert!(open.is_empty(), "the documentation leaves tags open: {:?}", open);

    // And the escaping, which is the failure a well-formedness check alone
    // would miss: a `<` in a doc comment must reach the page as text.
    assert!(
        doc.contains("&lt;") && doc.contains("&amp;"),
        "the sample's `<` and `&` did not reach the page escaped"
    );
    assert!(
        !doc.contains("<script>alert"),
        "a doc comment's angle brackets were emitted as markup"
    );
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
