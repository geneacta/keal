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
    let mut failed = 0;
    for dep in &m.deps {
        let into = deps_dir.join(&dep.name);
        match fetch_one(dep, &into) {
            Ok(what) => println!("{} {} ({} {})", what, dep.name, dep.at_key, dep.at),
            Err(e) => {
                eprintln!("error: {}", e);
                failed += 1;
            }
        }
    }
    if failed > 0 {
        eprintln!("{} of {} failed", failed, m.deps.len());
        return ExitCode::FAILURE;
    }
    let n = m.deps.len();
    println!("{} {} in {}", n, if n == 1 { "dependency" } else { "dependencies" }, deps_dir.display());
    ExitCode::SUCCESS
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
