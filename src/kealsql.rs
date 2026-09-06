//! `import "./blog.kealsql"` — the KealSql client module, generated on demand.
//!
//! A `.kealsql` file describes a PostgreSQL schema and its queries, and its
//! compiler answers a Keal module that talks to the database with those
//! queries already typed. Importing the `.kealsql` directly is the whole
//! point: a column renamed in the schema then breaks the program where it is
//! compiled, rather than the query where it runs.
//!
//! This mirrors `.jbind/` deliberately, down to the directory beside the file
//! and the rule that only the RUNNING commands generate — the dump commands
//! stay pure functions of what is on disk, or the self-hosting corpora would
//! be comparing different inputs on the two sides.
//!
//! One difference from `.jbind/`, and it is a decision rather than an
//! accident: this regenerates when the `.kealsql` is NEWER than the module,
//! not only when the module is missing. A Java class changes when somebody
//! upgrades a dependency; a schema changes while you are writing it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// The module an import of `file.kealsql` reads: `.kealsql/<stem>.client.keal`
/// beside it. `None` when the path names something else.
pub fn client_of(path: &Path) -> Option<PathBuf> {
    if path.extension().and_then(|e| e.to_str()) != Some("kealsql") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let dir = path.parent().unwrap_or(Path::new("."));
    Some(dir.join(".kealsql").join(format!("{}.client.keal", stem)))
}

/// The `.kealsql` a generated module came from — `client_of` backwards.
pub fn source_of(client: &Path) -> Option<PathBuf> {
    let name = client.file_name()?.to_str()?;
    let stem = name.strip_suffix(".client.keal")?;
    let dir = client.parent()?.parent()?;
    Some(dir.join(format!("{}.kealsql", stem)))
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// The compiler to run: `$KEALSQL` when set, `kealsql` on the path otherwise.
/// Named rather than searched for, so that a project pinning a build of it
/// says so in one place.
fn compiler() -> String {
    // An EMPTY variable counts as unset. `env::var` answers `Ok("")` for
    // `KEALSQL=`, which a plain `unwrap_or_else` takes as a path — and the
    // failure then says "`` is not installed", naming nothing. A shell
    // profile that unsets a variable by emptying it is ordinary, and this is
    // what every other tool does with an empty `JAVA_HOME`.
    match std::env::var("KEALSQL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => "kealsql".to_string(),
    }
}

/// Generates `client` from its `.kealsql` when it is missing or older than
/// the source. Does nothing, and says nothing, when it is already current.
pub fn ensure_client(client: &Path) -> Result<(), String> {
    let Some(source) = source_of(client) else {
        return Err(format!("`{}` does not name a KealSql client", client.display()));
    };
    if !source.exists() {
        return Err(format!(
            "`{}` is missing, and it is what `{}` is generated from",
            source.display(),
            client.display()
        ));
    }
    // Newer source, or no module at all. Equal timestamps count as current:
    // a file system with one-second resolution would otherwise regenerate on
    // every build of a project written in the same second.
    let current = match (mtime(&source), mtime(client)) {
        (Some(s), Some(c)) => c >= s,
        _ => false,
    };
    if current {
        return Ok(());
    }
    let dir = client.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create `{}`: {}", dir.display(), e))?;
    let tool = compiler();
    let out = Command::new(&tool).arg("--client").arg(dir).arg(&source).output();
    let out = match out {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "`{}` is not installed -- build it from https://github.com/geneacta/kealsql \
                 and put it on the path, or point `KEALSQL` at it",
                tool
            ))
        }
        Err(e) => return Err(format!("cannot run `{}`: {}", tool, e)),
    };
    if !out.status.success() {
        // Its errors are already one per line and already say where; passing
        // them through unchanged is better than wrapping them in ours.
        let said = String::from_utf8_lossy(&out.stdout);
        let said = if said.trim().is_empty() {
            String::from_utf8_lossy(&out.stderr).into_owned()
        } else {
            said.into_owned()
        };
        return Err(format!("`{}` refused `{}`:\n{}", tool, source.display(), said.trim_end()));
    }
    if !client.exists() {
        return Err(format!(
            "`{}` reported success and wrote no `{}`",
            tool,
            client.display()
        ));
    }
    Ok(())
}
