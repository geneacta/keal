//! `keal test` — run a project's Keal programs and say which ones lied.
//!
//! There is no framework here, and that is the design rather than an absence.
//! **A test is a program**: nothing to register, nothing to annotate, no name
//! a runner has to be taught to find. There are two rules and they fit in two
//! sentences.
//!
//! *Alone, a program must end at zero having printed nothing.* That is what
//! `assert` already gives, and it is why `tests/programs` holds 46 files and
//! not one snapshot.
//!
//! *With a `<name>.expected` beside it, it must answer exactly that.* And a
//! program's answer depends on how it ended: **one that succeeded is judged
//! on what it printed, one that failed is judged on why.** Standard output in
//! the first case, standard error in the second — never both, because a
//! program that ends at zero writes its result to output and its asides to
//! error, and one that stops writes the reason to error and leaves output
//! half-finished.
//!
//! That single rule is what all four of this repository's corpora were
//! already obeying: `tests/errors` and `tests/runtime` pin a failure,
//! `tests/native` and `tests/layout` pin an output, and `tests/programs` pins
//! silence. It also says, without a special case, why a compiler warning does
//! not fail a passing test — the program succeeded, so the warning is not its
//! answer.
//!
//! What is new is not the rules. It is that a project outside this one can
//! have them without writing the harness again — and three wrote it again,
//! which is the evidence this command exists on.
//!
//! **Every engine, and they must agree.** The tree-walker is the
//! specification and the bytecode VM must match it, so the default runs both
//! and a disagreement is a failure even when each engine passes on its own.
//! That is the property this language is built around, and a runner that
//! checked one engine would be a runner for some other language.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

/// How long one program gets before it is called stuck. A test that runs for
/// a minute is already telling you something; a test that never ends tells
/// you nothing at all, and takes the whole run down with it. This is the
/// number `--timeout` changes.
const LIMIT: Duration = Duration::from_secs(60);

/// Which engines a run asks for. The compiled one is opt-in because it needs
/// a C compiler, and a machine without one can still test everything else.
struct Engines {
    ast: bool,
    vm: bool,
    native: bool,
}

impl Engines {
    fn names(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.ast {
            v.push("--ast");
        }
        if self.vm {
            v.push("--vm");
        }
        if self.native {
            v.push("native");
        }
        v
    }
}

/// What one run of one file on one engine answered.
struct Ran {
    ok: bool,
    /// Killed for taking too long. Kept apart from every other failure
    /// because it is the only one where the program has not answered at all:
    /// what it had printed by then is a fragment, and comparing a fragment to
    /// a snapshot would report the wrong thing about the wrong file.
    stuck: bool,
    stdout: String,
    stderr: String,
    status: String,
}

impl Ran {
    /// What this program has to say for itself: its output when it worked,
    /// the reason when it did not.
    fn answer(&self) -> &str {
        if self.ok {
            &self.stdout
        } else {
            &self.stderr
        }
    }
}

pub fn run(args: &[String]) -> ExitCode {
    let mut engines = Engines { ast: false, vm: false, native: false };
    let mut update = false;
    let mut chosen = false;
    let mut limit = LIMIT;
    let mut want_timeout = false;
    let mut paths: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--ast" => {
                engines.ast = true;
                chosen = true;
            }
            "--vm" => {
                engines.vm = true;
                chosen = true;
            }
            "--native" => {
                engines.native = true;
                chosen = true;
            }
            "--update" => update = true,
            _ if want_timeout => {
                want_timeout = false;
                match a.parse::<u64>() {
                    Ok(n) if n > 0 => limit = Duration::from_secs(n),
                    _ => {
                        eprintln!("keal test: --timeout wants a number of seconds, not `{}`", a);
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--timeout" => want_timeout = true,
            other if other.starts_with('-') => {
                eprintln!("keal test: no such option `{}`", other);
                eprintln!(
                    "  = note: --ast, --vm, --native choose engines; --update rewrites \
                     `.expected` snapshots; --timeout <seconds> changes how long a \
                     program gets before it is called stuck"
                );
                return ExitCode::FAILURE;
            }
            other => paths.push(other.to_string()),
        }
    }
    if want_timeout {
        eprintln!("keal test: --timeout wants a number of seconds after it");
        return ExitCode::FAILURE;
    }
    // Both interpreters by default: they live in this binary, so asking for
    // both costs nothing but a second run, and their agreement is the thing
    // worth checking.
    if !chosen {
        engines.ast = true;
        engines.vm = true;
    }
    if paths.is_empty() {
        paths.push("tests".to_string());
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for p in &paths {
        let path = Path::new(p);
        if !path.exists() {
            eprintln!("keal test: `{}` is not there", p);
            return ExitCode::FAILURE;
        }
        collect(path, &mut files);
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        // Not a pass. A runner that finds nothing and says "ok" is the first
        // rung of the ladder in STATUS.md: nothing consumes the output.
        eprintln!("keal test: no `.keal` files under {}", paths.join(", "));
        return ExitCode::FAILURE;
    }

    let me = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("keal test: cannot find this program to run it again: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let mut failed = 0usize;
    let mut updated = 0usize;
    for file in &files {
        if let Some(why) = check_one(&me, file, &engines, update, limit, &mut updated) {
            failed += 1;
            println!("FAIL {}", shown(file));
            for line in why.lines() {
                println!("     {}", line);
            }
        }
    }

    let n = files.len();
    if updated > 0 {
        println!("{} snapshot(s) written", updated);
    }
    if failed == 0 {
        println!("{} file(s), all passed, on {}", n, engines.names().join(" "));
        ExitCode::SUCCESS
    } else {
        println!("{} file(s), {} failed, on {}", n, failed, engines.names().join(" "));
        ExitCode::FAILURE
    }
}

/// Every `.keal` under `path`, skipping the directories that hold somebody
/// else's code: a fetched dependency and a generated binding cache are not
/// this project's tests, and running them would report their failures as
/// ours.
fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("keal") {
            out.push(path.to_path_buf());
        }
        return;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if matches!(name, ".keal" | ".jbind" | ".kealsql" | "target" | ".git") {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else { return };
    let mut kids: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    kids.sort();
    for k in kids {
        collect(&k, out);
    }
}

/// `None` when the file passed everywhere it was asked to run.
fn check_one(
    me: &Path,
    file: &Path,
    engines: &Engines,
    update: bool,
    limit: Duration,
    updated: &mut usize,
) -> Option<String> {
    let expected_path = file.with_extension("expected");
    let expected = std::fs::read_to_string(&expected_path).ok();

    let mut results: Vec<(&'static str, Ran)> = Vec::new();
    if engines.ast {
        results.push(("--ast", interpret(me, file, "--ast", limit)));
    }
    if engines.vm {
        results.push(("--vm", interpret(me, file, "--vm", limit)));
    }
    if engines.native {
        match compiled(me, file, limit) {
            Ok(r) => results.push(("native", r)),
            Err(why) => return Some(why),
        }
    }

    // A program that had to be stopped is reported as that and nothing else.
    // Everything downstream compares answers, and this one has not given one.
    for (name, r) in &results {
        if r.stuck {
            return Some(format!(
                "{}: {}\n{}",
                name,
                r.status,
                indent("     = note: what it had printed by then is not an answer, so \
                        it was not compared; --timeout <seconds> allows longer"),
            ));
        }
    }

    // Every engine is asked before any is judged, so a disagreement is
    // reported as a disagreement rather than as whichever one ran first.
    let mut trouble = String::new();
    for w in results.windows(2) {
        let (a, ra) = &w[0];
        let (b, rb) = &w[1];
        if ra.ok != rb.ok {
            trouble += &format!(
                "{} and {} do not end the same way\n{}\n{}\n",
                a,
                b,
                indent(&format!("{}: ended {}", a, ra.status)),
                indent(&format!("{}: ended {}", b, rb.status)),
            );
        } else if ra.answer() != rb.answer() {
            trouble += &format!(
                "{} and {} do not answer the same thing\n{}\n{}\n",
                a,
                b,
                indent(&format!("{}: {}", a, one_line(ra.answer()))),
                indent(&format!("{}: {}", b, one_line(rb.answer()))),
            );
        }
    }
    if !trouble.is_empty() {
        return Some(trouble);
    }

    let first = &results[0].1;
    let answer = first.answer();
    let Some(want) = expected else {
        // No snapshot: the program is self-checking, which means silence and
        // zero. Output without a snapshot is either a forgotten `println` or
        // a snapshot nobody wrote, and both are worth stopping for.
        if !first.ok {
            return Some(format!(
                "it ended {}, and no `{}` says it should\n{}",
                first.status,
                shown(&expected_path),
                indent(answer.trim_end()),
            ));
        }
        if answer.is_empty() {
            return None;
        }
        if update {
            return write_snapshot(&expected_path, answer, updated);
        }
        return Some(format!(
            "it passed but printed something, and no `{}` says what\n{}\n{}",
            shown(&expected_path),
            indent(&one_line(answer)),
            "     = note: a self-checking program prints nothing; \
             `keal test --update` writes this down as a snapshot instead",
        ));
    };
    // An empty snapshot cannot be told from a missing one by reading it, and
    // the two mean different things. Refuse rather than guess which.
    if want.is_empty() {
        return Some(format!(
            "`{}` is empty, so it says nothing about this program\n{}",
            shown(&expected_path),
            "     = note: delete it to mean `must end at 0 in silence`",
        ));
    }
    if answer != want {
        if update {
            return write_snapshot(&expected_path, answer, updated);
        }
        return Some(format!(
            "what it answers is not what `{}` says\n{}\n{}",
            shown(&expected_path),
            indent(&format!("expected: {}", one_line(&want))),
            indent(&format!("     got: {}", one_line(answer))),
        ));
    }
    None
}

fn write_snapshot(path: &Path, said: &str, updated: &mut usize) -> Option<String> {
    match std::fs::write(path, said) {
        Ok(()) => {
            *updated += 1;
            None
        }
        Err(e) => Some(format!("cannot write {}: {}", shown(path), e)),
    }
}

fn interpret(me: &Path, file: &Path, engine: &str, limit: Duration) -> Ran {
    let mut cmd = Command::new(me);
    cmd.arg(engine).arg(file);
    bounded(cmd, limit)
}

/// Runs a command that must end on its own, and reports it as failed when it
/// does not.
///
/// Two things here are the point rather than plumbing. Standard input is
/// **closed**, not inherited: a test that reads it should see end-of-file,
/// and inheriting the terminal would leave the whole run waiting on a person
/// who is not there. And the wait has a deadline, because the one outcome a
/// runner must never produce is no outcome — a program still running after
/// `limit` is killed and written down as stuck, which is a result the next
/// file can be tested after.
fn bounded(mut cmd: Command, limit: Duration) -> Ran {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Ran {
                ok: false,
                stuck: false,
                stdout: String::new(),
                stderr: e.to_string(),
                status: "not at all".to_string(),
            }
        }
    };
    // The pipes are drained on their own threads from the moment the child
    // starts. Waiting first and reading after would deadlock on any program
    // that fills a pipe buffer, which is exactly the noisy program a timeout
    // is there to catch.
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());

    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break None,
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    match status {
        Some(st) => Ran {
            ok: st.success(),
            stuck: false,
            stdout,
            stderr,
            status: status_of(&st),
        },
        None => Ran {
            ok: false,
            stuck: true,
            stdout,
            stderr,
            status: format!("still running after {}s, and was stopped", limit.as_secs()),
        },
    }
}

fn drain<R: std::io::Read + Send + 'static>(
    pipe: Option<R>,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = std::io::Read::read_to_end(&mut p, &mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Builds the program and runs it.
///
/// A refusal the backend names — "the C backend cannot compile a method used
/// as a value yet" — is reported as a failure, and deliberately. It is not a
/// defect in the program, and on a run that did not ask for `--native` the
/// file passes. But someone who asked for the compiled engine asked a
/// question this file cannot answer, and the honest reply is red with the
/// refusal quoted, not a green that reads as coverage.
fn compiled(me: &Path, file: &Path, limit: Duration) -> Result<Ran, String> {
    let dir = std::env::temp_dir().join("keal-test-native");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Err(format!("cannot make a build directory: {}", e));
    }
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let exe = dir.join(format!("{}-{}", stem, std::process::id()));
    let mut build = Command::new(me);
    build.arg("build").arg(file).arg("-o").arg(&exe);
    // A build gets longer than a run: a C compiler on a cold cache is slow in
    // a way a test is not.
    let built = bounded(build, limit * 4);
    if !built.ok {
        let said = built.stderr.clone() + &built.stdout;
        if said.contains("cannot compile") {
            return Err(format!(
                "native: the backend refuses this program by name, so it was not run\n{}",
                indent(said.trim_end())
            ));
        }
        return Err(format!("native: it did not build\n{}", indent(said.trim_end())));
    }
    let ran = bounded(Command::new(&exe), limit);
    let _ = std::fs::remove_file(&exe);
    Ok(ran)
}

fn status_of(s: &std::process::ExitStatus) -> String {
    match s.code() {
        Some(c) => format!("at {}", c),
        None => format!("on a signal ({})", s),
    }
}

/// A first line and a count, so a report stays readable when a program that
/// should have printed nothing printed four hundred lines.
fn one_line(s: &str) -> String {
    let mut it = s.lines();
    let first = it.next().unwrap_or("");
    let rest = it.count();
    if rest == 0 {
        format!("{:?}", first)
    } else {
        format!("{:?} and {} more line(s)", first, rest)
    }
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("     {}", l)).collect::<Vec<_>>().join("\n")
}

fn shown(p: &Path) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    p.strip_prefix(&cwd).unwrap_or(p).display().to_string()
}
