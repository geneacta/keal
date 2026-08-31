//! `keal.toml` — what a project is called and what it depends on.
//!
//! The file is TOML, but only the shape this project needs is read: a
//! `[package]` table with `name` and `version`, and a `[dependencies]`
//! table whose values are inline tables naming a git repository and the
//! exact commit or tag to take from it. Anything else is skipped rather
//! than rejected, so a manifest may carry keys a later version will use.
//!
//! There is deliberately no resolution here and no registry: git already
//! provides the naming, the versioning and the immutability, and borrowing
//! them costs nothing. See `docs/packages.md` for the argument.

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Manifest {
    pub name: String,
    /// Read by nothing yet: a project's own version matters the day one
    /// project depends on another and the two have to be told apart.
    #[allow(dead_code)]
    pub version: String,
    pub deps: Vec<Dep>,
    /// The directory the manifest was found in: the project's root, and
    /// where `.keal/deps/` lives.
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub git: String,
    /// A tag or a commit — whichever the manifest named. Both are checked
    /// out by name, and both are recorded as written.
    pub at: String,
    /// Which key named it, for the message when it is missing.
    pub at_key: String,
}

/// Walks up from `start` for the nearest `keal.toml`. What `keal fetch`
/// wants: the project you are standing in.
pub fn find(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_dir() { start.to_path_buf() } else { start.parent()?.to_path_buf() };
    loop {
        let candidate = dir.join("keal.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// The OUTERMOST `keal.toml` above `start`: the project whose `.keal/deps`
/// everything shares.
///
/// This is what a `dep:` import resolves against, and the difference
/// matters exactly once dependencies have dependencies. A file inside
/// `.keal/deps/geometry/` has two manifests above it — geometry's own and
/// the project's — and its `dep:` imports must reach the one place every
/// dependency was fetched into, or a library would look for its own
/// dependencies inside itself and never find them.
pub fn root_of(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_dir() { start.to_path_buf() } else { start.parent()?.to_path_buf() };
    let mut outermost = None;
    loop {
        if dir.join("keal.toml").exists() {
            outermost = Some(dir.join("keal.toml"));
        }
        if !dir.pop() {
            return outermost;
        }
    }
}

pub fn read(path: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read `{}`: {}", path.display(), e))?;
    let root = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut name = String::new();
    let mut version = String::new();
    let mut deps = Vec::new();
    let mut table = String::new();

    for (i, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let Some(end) = line.find(']') else {
                return Err(at(path, i, "a table header needs a closing `]`"));
            };
            table = line[1..end].trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(at(path, i, "expected `key = value`"));
        };
        let key = key.trim().to_string();
        let value = value.trim();
        match table.as_str() {
            "package" => match key.as_str() {
                "name" => name = unquote(value).ok_or_else(|| at(path, i, "`name` must be a string"))?,
                "version" => {
                    version = unquote(value).ok_or_else(|| at(path, i, "`version` must be a string"))?
                }
                _ => {}
            },
            "dependencies" => {
                let dep = inline_dep(&key, value).ok_or_else(|| {
                    at(
                        path,
                        i,
                        "a dependency is `name = { git = \"...\", tag = \"...\" }` — or `rev` for a commit",
                    )
                })?;
                deps.push(dep);
            }
            _ => {}
        }
    }
    if name.is_empty() {
        return Err(format!("`{}` has no `name` under `[package]`", path.display()));
    }
    Ok(Manifest { name, version, deps, root })
}

fn at(path: &Path, line: usize, msg: &str) -> String {
    format!("{}:{}: {}", path.display(), line + 1, msg)
}

fn strip_comment(line: &str) -> &str {
    // A `#` inside quotes is content; anywhere else it starts a comment.
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(v: &str) -> Option<String> {
    let v = v.trim();
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.to_string())
}

fn inline_dep(name: &str, value: &str) -> Option<Dep> {
    let body = value.trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut git = String::new();
    let mut at = String::new();
    let mut at_key = String::new();
    for field in split_fields(body) {
        let (k, v) = field.split_once('=')?;
        let (k, v) = (k.trim(), unquote(v)?);
        match k {
            "git" => git = v,
            "tag" | "rev" => {
                at = v;
                at_key = k.to_string();
            }
            _ => {}
        }
    }
    if git.is_empty() || at.is_empty() {
        return None;
    }
    Some(Dep { name: name.to_string(), git, at, at_key })
}

/// Splits an inline table's fields on commas that are not inside a string.
fn split_fields(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    for c in body.chars() {
        match c {
            '"' => {
                in_str = !in_str;
                cur.push(c);
            }
            ',' if !in_str => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter().filter(|f| !f.trim().is_empty()).collect()
}
