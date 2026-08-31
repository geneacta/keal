//! `keal fetch` — puts a project's dependencies where its imports expect
//! them.
//!
//! Every dependency is a git repository at an exact tag or commit, cloned
//! into `.keal/deps/<name>/`. There is no resolution and no lockfile yet:
//! a manifest names commits, so what was fetched yesterday is what is
//! fetched today, and that is the whole promise this step makes.
//!
//! Nothing else in the compiler runs git. `import "dep:name/file.keal"`
//! reads what is on disk and says so plainly when it is not there, so a
//! committed `.keal/deps/` builds with no network at all.

use std::path::Path;
use std::process::{Command, ExitCode};

use crate::manifest::{self, Dep};

pub fn run(args: &[String]) -> ExitCode {
    if !args.is_empty() {
        eprintln!("usage: keal fetch");
        eprintln!("  = note: it reads `keal.toml` from this directory or the ones above it");
        return ExitCode::FAILURE;
    }
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read the current directory: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let Some(path) = manifest::find(&cwd) else {
        eprintln!("error: no `keal.toml` here or in any directory above");
        eprintln!("  = note: a project's manifest names it and lists what it depends on");
        return ExitCode::FAILURE;
    };
    let m = match manifest::read(&path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    if m.deps.is_empty() {
        println!("{} has no dependencies", m.name);
        return ExitCode::SUCCESS;
    }
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("error: `git` is not installed, and every dependency is a git repository");
        return ExitCode::FAILURE;
    }

    let deps_dir = m.root.join(".keal/deps");
    if let Err(e) = std::fs::create_dir_all(&deps_dir) {
        eprintln!("error: cannot create `{}`: {}", deps_dir.display(), e);
        return ExitCode::FAILURE;
    }

    // Every dependency lands in one directory, whoever asked for it — a
    // dependency's dependencies included. Flat rather than nested, because
    // two copies of a library are two different sets of types, and a
    // program that ends up holding both cannot say so.
    let mut queue: Vec<(Dep, String)> = m.deps.iter().map(|d| (d.clone(), m.name.clone())).collect();
    let mut resolved: Vec<(Dep, String)> = Vec::new();
    let mut failed = 0;
    while let Some((dep, asked_by)) = queue.pop() {
        // The same name twice is only a problem when the two disagree. We
        // pin commits rather than ranges, so there is nothing to reconcile:
        // say who wants what and stop.
        if let Some((seen, first)) = resolved.iter().find(|(d, _)| d.name == dep.name) {
            if seen.git != dep.git || seen.at != dep.at {
                eprintln!("error: two versions of `{}` are wanted, and only one can be here", dep.name);
                eprintln!("  = note: {} wants {} {} of {}", first, seen.at_key, seen.at, seen.git);
                eprintln!("  = note: {} wants {} {} of {}", asked_by, dep.at_key, dep.at, dep.git);
                eprintln!("  = note: commits are pinned, so nothing can pick between them: change one manifest");
                return ExitCode::FAILURE;
            }
            continue;
        }
        let into = deps_dir.join(&dep.name);
        match fetch_one(&dep, &into) {
            Ok(what) => println!("{} {} ({} {})", what, dep.name, dep.at_key, dep.at),
            Err(e) => {
                eprintln!("error: {}", e);
                failed += 1;
                continue;
            }
        }
        // What it depends on in turn. A dependency without a manifest is a
        // dependency with no dependencies, which is the common case and not
        // an error.
        let inner = into.join("keal.toml");
        if inner.exists() {
            match manifest::read(&inner) {
                Ok(sub) => {
                    for d in sub.deps {
                        queue.push((d, dep.name.clone()));
                    }
                }
                Err(e) => {
                    eprintln!("error: `{}` has an unreadable manifest: {}", dep.name, e);
                    failed += 1;
                }
            }
        }
        resolved.push((dep, asked_by));
    }
    if failed > 0 {
        eprintln!("{} of {} failed", failed, resolved.len() + failed);
        return ExitCode::FAILURE;
    }
    if let Err(e) = write_lock(&m.root, &resolved) {
        eprintln!("error: cannot write the lockfile: {}", e);
        return ExitCode::FAILURE;
    }
    let n = resolved.len();
    println!("{} {} in {}", n, if n == 1 { "dependency" } else { "dependencies" }, deps_dir.display());
    ExitCode::SUCCESS
}

/// Records what was actually fetched: the commit each name resolved to, and
/// who asked for it.
///
/// A manifest may name a tag, and a tag can be moved. This says which commit
/// the tag meant on the day it was read, so a checkout that carries the
/// lockfile is reproducible whatever the remote does afterwards.
fn write_lock(root: &Path, resolved: &[(Dep, String)]) -> std::io::Result<()> {
    let mut out = String::from(
        "# Written by `keal fetch`. It records the commit each dependency\n\
# resolved to, so that a tag moved upstream cannot silently change what\n\
# this project builds against. Commit it.\n",
    );
    let mut rows: Vec<&(Dep, String)> = resolved.iter().collect();
    rows.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    for (dep, asked_by) in rows {
        let at = deps_dir_commit(root, &dep.name).unwrap_or_else(|| dep.at.clone());
        out.push_str(&format!(
            "\n[[dependency]]\nname = \"{}\"\ngit = \"{}\"\n{} = \"{}\"\ncommit = \"{}\"\nasked_by = \"{}\"\n",
            dep.name, dep.git, dep.at_key, dep.at, at, asked_by
        ));
    }
    std::fs::write(root.join("keal.lock"), out)
}

/// The commit a fetched dependency is actually sitting on.
fn deps_dir_commit(root: &Path, name: &str) -> Option<String> {
    let dir = root.join(".keal/deps").join(name);
    let out = Command::new("git").args(["rev-parse", "HEAD"]).current_dir(dir).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Clones what is missing, and moves what is there to the commit named.
fn fetch_one(dep: &Dep, into: &Path) -> Result<&'static str, String> {
    let fresh = !into.join(".git").exists();
    if fresh {
        if into.exists() {
            return Err(format!(
                "`{}` exists but is not a git checkout; remove it and fetch again",
                into.display()
            ));
        }
        git(&["clone", "--quiet", &dep.git, &into.to_string_lossy()], None)
            .map_err(|e| format!("cannot clone `{}`: {}", dep.git, e))?;
    } else {
        // Already there: only the named commit has to be reachable, so a
        // plain fetch of everything is the honest way to make sure it is.
        git(&["fetch", "--quiet", "--tags", "origin"], Some(into))
            .map_err(|e| format!("cannot update `{}`: {}", dep.name, e))?;
    }
    git(&["checkout", "--quiet", "--detach", &dep.at], Some(into)).map_err(|e| {
        format!("`{}` has no {} `{}`: {}", dep.name, dep.at_key, dep.at, e)
    })?;
    Ok(if fresh { "cloned" } else { "updated" })
}

fn git(args: &[&str], in_dir: Option<&Path>) -> Result<(), String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(d) = in_dir {
        cmd.current_dir(d);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.trim().lines().last().unwrap_or("git failed").to_string())
}
