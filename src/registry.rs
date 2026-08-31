//! `keal search` and `keal add` — finding a package you do not already know
//! the URL of, and writing it down.
//!
//! `docs/packages.md` argued for a long time that a registry comes last, and
//! it gave the reason: a registry is a **service somebody has to run**, and a
//! compiler project should not be running one before it has users. That
//! argument stands, and this is not that. The index here is an ordinary git
//! repository holding one small file per package:
//!
//! ```toml
//! # packages/geometry.toml
//! [package]
//! name = "geometry"
//! git = "https://github.com/someone/geometry"
//! description = "points, lines and the arithmetic between them"
//! ```
//!
//! One file per package, so two people adding two packages never touch the
//! same line and a contribution is a pull request that adds a file. Git
//! provides the hosting, the history, the review and the immutability — the
//! same borrowing `keal fetch` already does, and it owes nobody a service to
//! keep running. If the index repository disappears tomorrow, every
//! `keal.toml` in the world still builds, because a manifest names the
//! package's own repository and never the index.
//!
//! **The index says where a package lives, and nothing else.** No versions,
//! no ranges, no resolution: `keal add` reads the URL from the index, asks
//! that repository what tags it has, and writes ONE exact pin into
//! `keal.toml`. What happens after that is what already happened before this
//! file existed. The one judgement it makes — which tag is newest — happens
//! once, at your keyboard, and is printed and written down. That is the
//! difference between a convenience and a resolver: a resolver decides again
//! on every build, under rules nobody read.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Where the index is read from when nothing says otherwise. `KEAL_INDEX`
/// overrides it — a fork, a company's own list, or a directory on disk for
/// a test.
const DEFAULT_INDEX: &str = "https://github.com/geneacta/keal-index";

/// One package, as the index describes it.
struct Entry {
    name: String,
    git: String,
    description: String,
}

pub fn search(args: &[String]) -> ExitCode {
    let terms: Vec<String> = args.iter().map(|a| a.to_lowercase()).collect();
    if terms.iter().any(|t| t.starts_with('-')) {
        eprintln!("usage: keal search [term...]");
        eprintln!("  = note: with no term it lists everything the index holds");
        return ExitCode::FAILURE;
    }
    let dir = match index_dir_ready() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let entries = match read_index(&dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let hits: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            terms.is_empty()
                || terms.iter().all(|t| {
                    e.name.to_lowercase().contains(t) || e.description.to_lowercase().contains(t)
                })
        })
        .collect();
    if hits.is_empty() {
        if entries.is_empty() {
            println!("the index at {} holds nothing yet", index_url());
        } else {
            println!("nothing in the index matches");
            println!("  = note: `keal index update` refreshes the local copy");
        }
        return ExitCode::SUCCESS;
    }
    let width = hits.iter().map(|e| e.name.len()).max().unwrap_or(0);
    for e in &hits {
        println!("{:width$}  {}", e.name, e.description, width = width);
        println!("{:width$}  {}", "", e.git, width = width);
    }
    println!();
    println!("{} of {} — `keal add <name>` writes one into keal.toml", hits.len(), entries.len());
    ExitCode::SUCCESS
}

pub fn add(args: &[String]) -> ExitCode {
    let [spec] = args else {
        eprintln!("usage: keal add <name>[@<tag>]");
        eprintln!("  = note: with no tag it takes the repository's newest version tag and writes it down");
        return ExitCode::FAILURE;
    };
    let (name, wanted) = match spec.split_once('@') {
        Some((n, t)) => (n.to_string(), Some(t.to_string())),
        None => (spec.clone(), None),
    };
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read the current directory: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let Some(path) = crate::manifest::find(&cwd) else {
        eprintln!("error: no `keal.toml` here or in any directory above");
        eprintln!("  = note: a package is added to a project, and a project is a manifest");
        return ExitCode::FAILURE;
    };
    let m = match crate::manifest::read(&path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    // Already named is not a mistake to fix silently: a pin is a decision,
    // and replacing one is an edit its author should make.
    if let Some(d) = m.deps.iter().find(|d| d.name == name) {
        eprintln!("error: `{}` is already a dependency of {}", name, m.name);
        eprintln!("  = note: it names {} {} of {}", d.at_key, d.at, d.git);
        eprintln!("  = note: change the line in `{}` to move it", path.display());
        return ExitCode::FAILURE;
    }

    let dir = match index_dir_ready() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let entries = match read_index(&dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let Some(entry) = entries.iter().find(|e| e.name == name) else {
        eprintln!("error: the index has no package called `{}`", name);
        eprintln!("  = note: `keal search {}` looks for one, and `keal index update` refreshes the local copy", name);
        eprintln!("  = note: a package not in the index still works: name its git URL in `keal.toml` directly");
        return ExitCode::FAILURE;
    };

    let (at_key, at) = match wanted {
        // A tag somebody typed is still checked before it is written: the
        // repository is already known, asking it costs one round trip, and
        // finding out here beats finding out from `keal fetch` later.
        Some(tag) => match confirm_tag(&entry.git, &tag) {
            Ok(()) => ("tag".to_string(), tag),
            Err(e) => {
                eprintln!("error: {}", e);
                return ExitCode::FAILURE;
            }
        },
        None => match newest_pin(&entry.git) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {}", e);
                return ExitCode::FAILURE;
            }
        },
    };
    if let Err(e) = write_dependency(&path, &name, &entry.git, &at_key, &at) {
        eprintln!("error: cannot write `{}`: {}", path.display(), e);
        return ExitCode::FAILURE;
    }
    println!("{} = {{ git = \"{}\", {} = \"{}\" }}", name, entry.git, at_key, at);
    println!("  written into {}", path.display());
    println!("  = note: `keal fetch` puts it where `import \"dep:{}/...\"` expects it", name);
    ExitCode::SUCCESS
}

pub fn index(args: &[String]) -> ExitCode {
    match args.first().map(|s| s.as_str()) {
        Some("update") => match update_index() {
            Ok(dir) => {
                println!("index updated from {}", index_url());
                println!("  {}", dir.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {}", e);
                ExitCode::FAILURE
            }
        },
        Some("path") => {
            match index_dir() {
                Ok(d) => println!("{}", d.display()),
                Err(e) => {
                    eprintln!("error: {}", e);
                    return ExitCode::FAILURE;
                }
            }
            println!("  from {}", index_url());
            ExitCode::SUCCESS
        }
        // The file a contributor adds to the index. It is printed rather
        // than pushed: publishing is a pull request somebody reviews, which
        // is the whole reason the index is a git repository.
        Some("entry") => entry_for_this_project(),
        _ => {
            eprintln!("usage: keal index <update|path|entry>");
            eprintln!("  update  refresh the local copy of the package index");
            eprintln!("  path    say where that copy is");
            eprintln!("  entry   print the index file this project would contribute");
            ExitCode::FAILURE
        }
    }
}

fn entry_for_this_project() -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read the current directory: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let Some(path) = crate::manifest::find(&cwd) else {
        eprintln!("error: no `keal.toml` here or in any directory above");
        return ExitCode::FAILURE;
    };
    let m = match crate::manifest::read(&path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };
    // The project's own remote, if git knows one — it is the URL other
    // people will clone, and guessing it wrong is worse than leaving it
    // blank for a person to fill in.
    let git = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(&m.root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "https://…".to_string());
    println!("# packages/{}.toml", m.name);
    println!("[package]");
    println!("name = \"{}\"", m.name);
    println!("git = \"{}\"", git);
    println!("description = \"…one line, what it is for\"");
    println!();
    println!("# Add that file to {} and open a pull request.", index_url());
    println!("# The index says where a package lives; its versions stay its own git tags.");
    ExitCode::SUCCESS
}

fn index_url() -> String {
    std::env::var("KEAL_INDEX").unwrap_or_else(|_| DEFAULT_INDEX.to_string())
}

/// Where the local copy of the index lives. Beside the user, not beside the
/// project: it is a cache of a shared list, and every project reads the same
/// one.
fn index_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("KEAL_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .ok_or_else(|| {
            "cannot tell where your home directory is; set `KEAL_HOME` to say where the index should live".to_string()
        })?;
    Ok(home.join(".keal").join("index"))
}

/// The index, cloned if it is not there yet — and **not** updated if it is.
/// A command that quietly refreshed would mean two runs of `keal add` on the
/// same day could write different things; `keal index update` is the one
/// that changes what you are reading, and you ask for it.
fn index_dir_ready() -> Result<PathBuf, String> {
    let dir = index_dir()?;
    if dir.join(".git").exists() || dir.join("packages").exists() {
        return Ok(dir);
    }
    clone_index(&dir)?;
    println!("index cloned from {}", index_url());
    Ok(dir)
}

fn clone_index(dir: &Path) -> Result<(), String> {
    if Command::new("git").arg("--version").output().is_err() {
        return Err("`git` is not installed, and the package index is a git repository".to_string());
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create `{}`: {}", parent.display(), e))?;
    }
    let url = index_url();
    git(&["clone", "--quiet", "--depth", "1", &url, &dir.to_string_lossy()], None)
        .map_err(|e| format!("cannot clone the index from `{}`: {}", url, e))
}

fn update_index() -> Result<PathBuf, String> {
    let dir = index_dir()?;
    if !dir.join(".git").exists() {
        clone_index(&dir)?;
        return Ok(dir);
    }
    git(&["fetch", "--quiet", "--depth", "1", "origin", "HEAD"], Some(&dir))
        .map_err(|e| format!("cannot reach the index at `{}`: {}", index_url(), e))?;
    git(&["reset", "--quiet", "--hard", "FETCH_HEAD"], Some(&dir))
        .map_err(|e| format!("cannot update the local index: {}", e))?;
    Ok(dir)
}

/// Every `packages/*.toml` the index holds, in name order.
///
/// A file that does not parse is named and skipped rather than fatal: one
/// bad contribution must not make the whole index unreadable for everybody
/// standing behind it.
fn read_index(dir: &Path) -> Result<Vec<Entry>, String> {
    let packages = dir.join("packages");
    if !packages.is_dir() {
        return Err(format!(
            "`{}` has no `packages/` directory, so it is not a Keal index",
            dir.display()
        ));
    }
    let read = std::fs::read_dir(&packages)
        .map_err(|e| format!("cannot read `{}`: {}", packages.display(), e))?;
    let mut out = Vec::new();
    for item in read.flatten() {
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        match parse_entry(&text) {
            Some(e) => out.push(e),
            None => eprintln!(
                "warning: `{}` is not a package entry, and is skipped",
                path.display()
            ),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The three keys an entry has. Written by hand, so it is read the way the
/// manifest reader reads a manifest: the shape that is needed, and anything
/// else skipped rather than rejected.
fn parse_entry(text: &str) -> Option<Entry> {
    let mut name = String::new();
    let mut git = String::new();
    let mut description = String::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().trim_matches('"').to_string();
        match k.trim() {
            "name" => name = v,
            "git" => git = v,
            "description" => description = v,
            _ => {}
        }
    }
    if name.is_empty() || git.is_empty() {
        return None;
    }
    Some(Entry { name, git, description })
}

/// The pin `keal add` writes when nobody named one: the repository's newest
/// version tag, or its current commit when it has none.
///
/// "Newest" means the highest tag spelled as digits and dots, with an
/// optional leading `v`, compared number by number. Anything else — `nightly`,
/// `v2.0-rc1`, a branch name — is not a version and is not considered. The
/// rule is small on purpose, and it runs **once**: what it picks is written
/// into the manifest as an exact tag, so no later build repeats the choice.
fn newest_pin(git_url: &str) -> Result<(String, String), String> {
    let mut best: Option<(Vec<u64>, String)> = None;
    for tag in remote_tags(git_url)? {
        let Some(parts) = version_parts(&tag) else { continue };
        let better = match &best {
            None => true,
            Some((seen, _)) => compare_parts(&parts, seen) == std::cmp::Ordering::Greater,
        };
        if better {
            best = Some((parts, tag));
        }
    }
    if let Some((_, tag)) = best {
        return Ok(("tag".to_string(), tag));
    }
    // No version tag at all. A commit is still an exact pin, and pinning one
    // is better than pinning a branch, which moves.
    let out = Command::new("git")
        .args(["ls-remote", git_url, "HEAD"])
        .output()
        .map_err(|e| format!("cannot reach `{}`: {}", git_url, e))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let commit = text.split_whitespace().next().unwrap_or("").to_string();
    if commit.is_empty() {
        return Err(format!("`{}` has no version tags and no commits to pin", git_url));
    }
    eprintln!("note: `{}` has no version tags, so its current commit is pinned instead", git_url);
    Ok(("rev".to_string(), commit))
}

/// Whether a repository has this tag, and what it does have when it does
/// not. A pin nobody can check out is not a pin.
fn confirm_tag(git_url: &str, tag: &str) -> Result<(), String> {
    let tags = remote_tags(git_url)?;
    if tags.iter().any(|t| t == tag) {
        return Ok(());
    }
    let mut versions: Vec<&String> = tags.iter().filter(|t| version_parts(t).is_some()).collect();
    versions.sort_by(|a, b| compare_parts(&version_parts(a).unwrap(), &version_parts(b).unwrap()));
    let recent: Vec<&str> = versions.iter().rev().take(5).map(|s| s.as_str()).collect();
    if recent.is_empty() {
        return Err(format!("`{}` has no tag `{}`, and no version tags at all", git_url, tag));
    }
    Err(format!(
        "`{}` has no tag `{}`\n  = note: its newest are {}",
        git_url,
        tag,
        recent.join(", ")
    ))
}

/// Every tag a repository publishes, asked once.
fn remote_tags(git_url: &str) -> Result<Vec<String>, String> {
    if Command::new("git").arg("--version").output().is_err() {
        return Err("`git` is not installed, and a package is a git repository".to_string());
    }
    let out = Command::new("git")
        .args(["ls-remote", "--tags", "--refs", git_url])
        .output()
        .map_err(|e| format!("cannot reach `{}`: {}", git_url, e))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "cannot read the tags of `{}`: {}",
            git_url,
            err.trim().lines().last().unwrap_or("git failed")
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .filter_map(|l| l.split("refs/tags/").nth(1))
        .map(|t| t.trim().to_string())
        .collect())
}

/// `v1.2.3` and `1.2.3` are versions; anything with another character in it
/// is not. Returning `None` is how a tag says "I am not a version".
fn version_parts(tag: &str) -> Option<Vec<u64>> {
    let body = tag.strip_prefix('v').unwrap_or(tag);
    if body.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for piece in body.split('.') {
        if piece.is_empty() || !piece.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        out.push(piece.parse::<u64>().ok()?);
    }
    Some(out)
}

/// Number by number, and a longer version wins a tie: `1.2.1` is above
/// `1.2`, as everybody reading it expects.
fn compare_parts(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Adds one line to `[dependencies]`, and nothing else. The rest of the file
/// comes out byte for byte as it went in: a manifest is a person's file, and
/// a tool that reformats it on the way past has taken something.
fn write_dependency(
    path: &Path,
    name: &str,
    git_url: &str,
    at_key: &str,
    at: &str,
) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let line = format!("{} = {{ git = \"{}\", {} = \"{}\" }}", name, git_url, at_key, at);
    let ends_with_newline = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

    // Under the existing `[dependencies]`, after its last entry — or a new
    // table at the end when there is none.
    let header = lines.iter().position(|l| l.trim() == "[dependencies]");
    match header {
        Some(start) => {
            let mut at_line = start + 1;
            let mut i = start + 1;
            while i < lines.len() {
                let t = lines[i].trim();
                if t.starts_with('[') {
                    break;
                }
                if !t.is_empty() {
                    at_line = i + 1;
                }
                i += 1;
            }
            lines.insert(at_line, line);
        }
        None => {
            if lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
            lines.push("[dependencies]".to_string());
            lines.push(line);
        }
    }
    let mut out = lines.join("\n");
    if ends_with_newline || !out.is_empty() {
        out.push('\n');
    }
    std::fs::write(path, out)
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
