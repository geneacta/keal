//! `keal jbind java.time.LocalDate` — Java classes in, a typed Keal module
//! over the JVM gateway (lib/jvm.keal) out.
//!
//! `javap -public` describes each class; every member whose types Keal can
//! carry across JNI exactly becomes a wrapper — instance methods on a
//! handle-holding class, statics and constructors as free functions — and
//! everything else is **skipped with the reason printed**, the bindgen
//! rule: a guessed binding is a crash with a delay.
//!
//! What crosses: `int`, `long`, `double`, `boolean`, `void` returns,
//! `java.lang.String`, and any class bound in the same run (so bind
//! `java.time.DayOfWeek` alongside `LocalDate` and `getDayOfWeek()`
//! comes typed). Kotlin classes are plain JVM classes: same generator.
//!
//! An argument naming an existing file is read as saved `javap` output
//! instead of running `javap` — that keeps the snapshot test JDK-free.

use std::path::Path;
use std::process::{Command, ExitCode};

/// The gateway module, embedded so a generated cache is self-contained: a
/// `.jbind/` directory carries its own copy and imports it as `jvm.keal`.
const JVM_KEAL: &str = include_str!("../lib/jvm.keal");

pub fn run(args: &[String]) -> ExitCode {
    let mut jvm_path = "lib/jvm.keal".to_string();
    let mut cache_dir: Option<String> = None;
    let mut inputs: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--jvm" {
            let Some(p) = args.get(i + 1) else {
                eprintln!("error: `--jvm` needs the import path to lib/jvm.keal");
                return ExitCode::FAILURE;
            };
            jvm_path = p.clone();
            i += 2;
        } else if args[i] == "--cache" {
            let Some(p) = args.get(i + 1) else {
                eprintln!("error: `--cache` needs a directory (usually `.jbind`)");
                return ExitCode::FAILURE;
            };
            cache_dir = Some(p.clone());
            i += 2;
        } else {
            inputs.push(args[i].clone());
            i += 1;
        }
    }
    if inputs.is_empty() {
        eprintln!("error: `keal jbind` needs at least one Java class name");
        return ExitCode::FAILURE;
    }

    if let Some(dir) = cache_dir {
        // The same file an `import java.time.LocalDate, ...` would load.
        let target = Path::new(&dir).join(format!("{}.keal", inputs.join("+")));
        return match ensure_cache(&target) {
            Ok(()) => {
                println!("{}", target.display());
                ExitCode::SUCCESS
            }
            Err(reason) => {
                eprintln!("error: {}", reason);
                ExitCode::FAILURE
            }
        };
    }

    let mut texts = Vec::new();
    for input in &inputs {
        match fetch(input) {
            Ok(t) => texts.push(t),
            Err(reason) => {
                eprintln!("error: {}", reason);
                return ExitCode::FAILURE;
            }
        }
    }
    print!("{}", generate(&jvm_path, &inputs, &texts));
    ExitCode::SUCCESS
}

/// One input's `javap` text: an existing file is read as saved output,
/// anything else is asked of the JDK.
fn fetch(input: &str) -> Result<String, String> {
    if Path::new(input).is_file() {
        return std::fs::read_to_string(input)
            .map_err(|e| format!("cannot read `{}`: {}", input, e));
    }
    let out = Command::new("javap")
        .arg("-public")
        .arg(input)
        .output()
        .map_err(|e| format!("cannot run `javap` (jbind needs a JDK): {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "`javap -public {}` failed:\n{}",
            input,
            String::from_utf8_lossy(&out.stderr).trim_end()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Writes the module behind one `import java.time.LocalDate, ...` — `path`
/// is the `.jbind/<classes joined with +>.keal` the import desugared to.
/// The directory gets its own copy of the gateway to stay self-contained.
pub fn ensure_cache(path: &Path) -> Result<(), String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("`{}` does not name a module", path.display()))?;
    let inputs: Vec<String> = stem.split('+').map(str::to_string).collect();
    let mut texts = Vec::new();
    for input in &inputs {
        texts.push(fetch(input)?);
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create `{}`: {}", dir.display(), e))?;
    let gateway = dir.join("jvm.keal");
    if !gateway.exists() {
        std::fs::write(&gateway, JVM_KEAL)
            .map_err(|e| format!("cannot write `{}`: {}", gateway.display(), e))?;
    }
    std::fs::write(path, generate("jvm.keal", &inputs, &texts))
        .map_err(|e| format!("cannot write `{}`: {}", path.display(), e))
}

const KEAL_KEYWORDS: &[&str] = &[
    "val", "var", "fun", "proc", "class", "if", "else", "unless", "when", "is", "in", "for",
    "while", "return", "break", "continue", "null", "true", "false", "this", "import", "not",
    "and", "or", "xor", "xnor", "nand", "nor", "implies", "borrow", "own",
];

/// Names the wrapper takes for itself on every class.
const RESERVED: &[&str] = &["free", "handle"];

pub fn generate(jvm_path: &str, inputs: &[String], texts: &[String]) -> String {
    let parsed: Vec<ParsedClass> = texts.iter().map(|t| parse_javap(t)).collect();

    // The classes bound in this run see each other typed.
    let mut bound: Vec<(String, String)> = Vec::new(); // (fqcn, simple)
    for p in &parsed {
        let simple = simple_name(&p.fqcn);
        if !is_keal_ident(&simple) {
            continue;
        }
        if !bound.iter().any(|(_, s)| *s == simple) {
            bound.push((p.fqcn.clone(), simple));
        }
    }

    let mut out = String::new();
    out.push_str(&format!(
        "// Generated by `keal jbind {}`. Typed wrappers over the JVM\n\
         // gateway: call `jvmStart` before anything here, and `free()` each\n\
         // wrapper when done — handles are JNI global references. Members\n\
         // whose types cannot cross are skipped below, with the reason.\n\n\
         import \"{}\"\n",
        inputs.join(" "),
        jvm_path
    ));

    for p in &parsed {
        let simple = simple_name(&p.fqcn);
        if !is_keal_ident(&simple) {
            out.push_str(&format!(
                "\n// skipped class {}: `{}` is not a Keal identifier\n",
                p.fqcn, simple
            ));
            continue;
        }
        if bound.iter().find(|(f, s)| *s == simple && *f != p.fqcn).is_some() {
            out.push_str(&format!(
                "\n// skipped class {}: another bound class is already named `{}`\n",
                p.fqcn, simple
            ));
            continue;
        }
        out.push_str(&emit_class(p, &simple, &bound));
    }
    out
}

struct ParsedClass {
    fqcn: String,
    members: Vec<String>,
}

/// The class header names the class; every following line up to the closing
/// brace is one member declaration.
fn parse_javap(text: &str) -> ParsedClass {
    let mut fqcn = String::new();
    let mut members = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("Compiled from") || line == "}" {
            continue;
        }
        if line.ends_with('{') {
            let words: Vec<&str> = line.trim_end_matches('{').split_whitespace().collect();
            for (i, w) in words.iter().enumerate() {
                if matches!(*w, "class" | "interface" | "enum") {
                    if let Some(name) = words.get(i + 1) {
                        fqcn = name.split('<').next().unwrap_or(name).to_string();
                    }
                    break;
                }
            }
            continue;
        }
        members.push(line.trim_end_matches(';').to_string());
    }
    ParsedClass { fqcn, members }
}

fn simple_name(fqcn: &str) -> String {
    fqcn.rsplit('.').next().unwrap_or(fqcn).to_string()
}

fn is_keal_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().unwrap().is_ascii_alphabetic()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEAL_KEYWORDS.contains(&s)
}

/// `UUID` → `uuid`, `LocalDate` → `localDate`: the leading uppercase run
/// drops, keeping its last letter capital when a word follows it.
fn lower_camel(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut run = 0;
    while run < chars.len() && chars[run].is_ascii_uppercase() {
        run += 1;
    }
    let lower_upto = if run > 1 && run < chars.len() { run - 1 } else { run };
    chars[..lower_upto].iter().map(|c| c.to_ascii_lowercase()).chain(chars[lower_upto..].iter().copied()).collect()
}

fn upper_first(s: &str) -> String {
    let mut cs = s.chars();
    match cs.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + cs.as_str(),
        None => String::new(),
    }
}

/// A Java type Keal can carry across JNI, with everything emission needs.
struct Crossing {
    keal: String,
    jni: String,
    /// The `jvmArg*` push for a parameter named `name`.
    push: fn(&str) -> String,
    ret: RetKind,
}

enum RetKind {
    Unit,
    Int,
    Long,
    Double,
    Bool,
    Str,
    Obj(String),
}

fn crossing(ty: &str, bound: &[(String, String)]) -> Result<Crossing, String> {
    if ty.contains('<') || ty.contains('>') {
        return Err(format!("`{}` is generic, and generics do not cross", ty));
    }
    if ty.ends_with("[]") {
        return Err(format!("`{}` is an array, and arrays do not cross yet", ty));
    }
    Ok(match ty {
        "int" => Crossing {
            keal: "Int".into(),
            jni: "I".into(),
            push: |n| format!("jvmArgInt({})", n),
            ret: RetKind::Int,
        },
        "long" => Crossing {
            keal: "Int".into(),
            jni: "J".into(),
            push: |n| format!("jvmArgLong({})", n),
            ret: RetKind::Long,
        },
        "double" => Crossing {
            keal: "Float".into(),
            jni: "D".into(),
            push: |n| format!("jvmArgDouble({})", n),
            ret: RetKind::Double,
        },
        "boolean" => Crossing {
            keal: "Bool".into(),
            jni: "Z".into(),
            push: |n| format!("jvmArgBool({})", n),
            ret: RetKind::Bool,
        },
        "void" => Crossing {
            keal: "Unit".into(),
            jni: "V".into(),
            push: |_| unreachable!("a void parameter does not exist in Java"),
            ret: RetKind::Unit,
        },
        "java.lang.String" => Crossing {
            keal: "String".into(),
            jni: "Ljava/lang/String;".into(),
            push: |n| format!("jvmArgStr({})", n),
            ret: RetKind::Str,
        },
        _ => match bound.iter().find(|(f, _)| f == ty) {
            Some((fqcn, simple)) => Crossing {
                keal: simple.clone(),
                jni: format!("L{};", fqcn.replace('.', "/")),
                push: |n| format!("jvmArgObj({}.handle)", n),
                ret: RetKind::Obj(simple.clone()),
            },
            None => {
                return Err(format!(
                    "`{}` does not cross — bind it in the same run and it will",
                    ty
                ))
            }
        },
    })
}

/// One javap member line, split into what emission needs.
enum Member {
    Field(String),
    Ctor(Vec<String>),
    Method { is_static: bool, ret: String, name: String, params: Vec<String> },
    Unreadable(String),
}

fn parse_member(line: &str, fqcn: &str) -> Member {
    let line = match line.find(" throws ") {
        Some(i) => &line[..i],
        None => line,
    };
    let Some(open) = line.find('(') else {
        // No parentheses: a field. Its name is the last word.
        let name = line.split_whitespace().last().unwrap_or("?").to_string();
        return Member::Field(name);
    };
    let Some(close) = line.rfind(')') else {
        return Member::Unreadable(line.to_string());
    };
    let head: Vec<&str> = line[..open]
        .split_whitespace()
        .filter(|w| {
            !matches!(
                *w,
                "public" | "protected" | "private" | "static" | "final" | "abstract" | "native"
                    | "synchronized" | "default" | "strictfp"
            )
        })
        .collect();
    let is_static = line[..open].split_whitespace().any(|w| w == "static");
    let params: Vec<String> = {
        let inner = line[open + 1..close].trim();
        if inner.is_empty() {
            Vec::new()
        } else {
            split_params(inner)
        }
    };
    match head.as_slice() {
        [ctor] if *ctor == fqcn => Member::Ctor(params),
        [ret, name] => Member::Method {
            is_static,
            ret: (*ret).to_string(),
            name: (*name).to_string(),
            params,
        },
        _ => Member::Unreadable(line.trim().to_string()),
    }
}

/// Splits `int, java.util.Map<a, b>, long` at the commas outside `<>`.
fn split_params(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in inner.chars() {
        match c {
            '<' => {
                depth += 1;
                cur.push(c);
            }
            '>' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// One wrapper's worth of text: what goes inside the class, what follows it,
/// or why nothing does.
enum Emitted {
    Instance(String),
    Toplevel(String),
    Skipped(String, String),
    Ignored,
}

fn emit_class(p: &ParsedClass, simple: &str, bound: &[(String, String)]) -> String {
    let lc = lower_camel(simple);
    let accessor = format!("jbindClass{}", simple);
    // A representable `compareTo(Self): Int` makes the wrapper `Ord`, so
    // prelude `compare` and the comparison operators reach across the JVM.
    let is_ord = p.members.iter().any(|m| match parse_member(m, &p.fqcn) {
        Member::Method { is_static: false, ret, name, params } => {
            name == "compareTo" && ret == "int" && params.len() == 1 && params[0] == p.fqcn
        }
        _ => false,
    });
    let mut instance = Vec::new();
    let mut toplevel = Vec::new();
    let mut skips: Vec<(String, String)> = Vec::new();
    let mut seen_instance: Vec<String> = vec!["toString".to_string()];
    let mut seen_toplevel: Vec<String> = Vec::new();

    for m in &p.members {
        match emit_member(m, p, simple, &lc, &accessor, bound, &mut seen_instance, &mut seen_toplevel) {
            Emitted::Instance(text) => instance.push(text),
            Emitted::Toplevel(text) => toplevel.push(text),
            Emitted::Skipped(label, reason) => skips.push((label, reason)),
            Emitted::Ignored => {}
        }
    }

    let mut out = String::new();
    out.push_str(&format!("\n// ---- {} ----\n\n", p.fqcn));
    out.push_str(&format!(
        "var jbindCls{} = 0\n\
         public fun {}(): Int {{\n    \
         if (jbindCls{} == 0) {{ jbindCls{} = jvmClass(\"{}\") }}\n    \
         return jbindCls{}\n\
         }}\n\n",
        simple,
        accessor,
        simple,
        simple,
        p.fqcn.replace('.', "/"),
        simple
    ));
    out.push_str(&format!(
        "public class {}(val handle: Int){} {{\n",
        simple,
        if is_ord { " : Ord" } else { "" }
    ));
    out.push_str("    var released: Bool = false\n");
    out.push_str("    fun toString(): String { return jvmToString(this.handle) }\n");
    out.push_str(
        "    proc free() {\n        unless (this.released) {\n            this.released = true\n            jvmFree(this.handle)\n        }\n    }\n",
    );
    out.push_str("    proc deinit() { this.free() }\n");
    for text in &instance {
        out.push_str(text);
    }
    out.push_str("}\n");
    for text in &toplevel {
        out.push('\n');
        out.push_str(text);
    }
    for (label, reason) in &skips {
        out.push('\n');
        out.push_str(&format!("// skipped {}: {}\n", label, reason));
    }
    out
}

fn emit_member(
    line: &str,
    p: &ParsedClass,
    simple: &str,
    lc: &str,
    accessor: &str,
    bound: &[(String, String)],
    seen_instance: &mut Vec<String>,
    seen_toplevel: &mut Vec<String>,
) -> Emitted {
    match parse_member(line, &p.fqcn) {
        Member::Field(name) => Emitted::Skipped(name, "fields are not bound yet".into()),
        Member::Unreadable(text) => {
            Emitted::Skipped(format!("`{}`", text), "this declaration did not parse".into())
        }
        Member::Ctor(params) => {
            let label = format!("{}({})", simple_name(&p.fqcn), params.join(", "));
            let fname = format!("{}New", lc);
            let crossed = match cross_all(&params, bound) {
                Ok(c) => c,
                Err(reason) => return Emitted::Skipped(label, reason),
            };
            if seen_toplevel.contains(&fname) {
                return Emitted::Skipped(
                    label,
                    "overloads an emitted constructor; call it through lib/jvm.keal directly"
                        .into(),
                );
            }
            seen_toplevel.push(fname.clone());
            let sig = format!("({})V", crossed.iter().map(|c| c.jni.as_str()).collect::<String>());
            let mut body = pushes(&crossed);
            body.push_str(&format!(
                "    return {}(jvmNew({}(), \"{}\"))\n",
                simple, accessor, sig
            ));
            Emitted::Toplevel(format!(
                "public fun {}({}): {} {{\n{}}}\n",
                fname,
                params_list(&crossed),
                simple,
                body
            ))
        }
        Member::Method { is_static, ret, name, params } => {
            let label = format!("{}({})", name, params.join(", "));
            if name == "toString" && params.is_empty() {
                return Emitted::Ignored; // the wrapper already answers it
            }
            if !is_static && RESERVED.contains(&name.as_str()) {
                return Emitted::Skipped(label, format!("the wrapper reserves `{}`", name));
            }
            if !is_keal_ident(&name) {
                return Emitted::Skipped(label, "its name is not a Keal identifier".into());
            }
            let crossed = match cross_all(&params, bound) {
                Ok(c) => c,
                Err(reason) => return Emitted::Skipped(label, reason),
            };
            let rk = match crossing(&ret, bound) {
                Ok(c) => c.ret,
                Err(reason) => return Emitted::Skipped(label, reason),
            };
            let sig = format!(
                "({}){}",
                crossed.iter().map(|c| c.jni.as_str()).collect::<String>(),
                jni_of(&ret, bound)
            );
            if is_static {
                let fname = format!("{}{}", lc, upper_first(&name));
                if seen_toplevel.contains(&fname) {
                    return Emitted::Skipped(
                        label,
                        "overloads an emitted method; call it through lib/jvm.keal directly".into(),
                    );
                }
                seen_toplevel.push(fname.clone());
                let recv = format!("{}()", accessor);
                let (kw, ret_ty, body) = call(&rk, &recv, &name, &sig, true, &crossed);
                Emitted::Toplevel(format!(
                    "public {} {}({}){} {{\n{}}}\n",
                    kw,
                    fname,
                    params_list(&crossed),
                    ret_ty,
                    body
                ))
            } else {
                if seen_instance.contains(&name) {
                    return Emitted::Skipped(
                        label,
                        "overloads an emitted method; call it through lib/jvm.keal directly".into(),
                    );
                }
                seen_instance.push(name.clone());
                let (kw, ret_ty, body) = call(&rk, "this.handle", &name, &sig, false, &crossed);
                let indented: String =
                    body.lines().map(|l| format!("    {}\n", l)).collect();
                Emitted::Instance(format!(
                    "    {} {}({}){} {{\n{}    }}\n",
                    kw,
                    name,
                    params_list(&crossed),
                    ret_ty,
                    indented
                ))
            }
        }
    }
}

fn cross_all(params: &[String], bound: &[(String, String)]) -> Result<Vec<Crossing>, String> {
    params.iter().map(|p| crossing(p, bound)).collect()
}

fn jni_of(ty: &str, bound: &[(String, String)]) -> String {
    crossing(ty, bound).map(|c| c.jni).unwrap_or_default()
}

fn params_list(crossed: &[Crossing]) -> String {
    crossed
        .iter()
        .enumerate()
        .map(|(i, c)| format!("a{}: {}", i, c.keal))
        .collect::<Vec<_>>()
        .join(", ")
}

fn pushes(crossed: &[Crossing]) -> String {
    crossed
        .iter()
        .enumerate()
        .map(|(i, c)| format!("    {}\n", (c.push)(&format!("a{}", i))))
        .collect()
}

/// The keyword, return annotation and body of one wrapper, chosen by what
/// the Java method gives back. JNI requires the call to match the exact
/// return type, so `int` and `long` take different gateway calls.
fn call(
    rk: &RetKind,
    recv: &str,
    name: &str,
    sig: &str,
    is_static: bool,
    crossed: &[Crossing],
) -> (&'static str, String, String) {
    let gate = |kind: &str| {
        format!(
            "jvm{}{}({}, \"{}\", \"{}\")",
            if is_static { "Static" } else { "Call" },
            kind,
            recv,
            name,
            sig
        )
    };
    let mut body = pushes(crossed);
    match rk {
        RetKind::Unit => {
            body.push_str(&format!("    {}\n", gate("Void")));
            ("proc", String::new(), body)
        }
        RetKind::Int => {
            body.push_str(&format!("    return {}\n", gate("Int")));
            ("fun", ": Int".into(), body)
        }
        RetKind::Long => {
            body.push_str(&format!("    return {}\n", gate("Long")));
            ("fun", ": Int".into(), body)
        }
        RetKind::Double => {
            body.push_str(&format!("    return {}\n", gate("Double")));
            ("fun", ": Float".into(), body)
        }
        RetKind::Bool => {
            body.push_str(&format!("    return {}\n", gate("Bool")));
            ("fun", ": Bool".into(), body)
        }
        RetKind::Str => {
            body.push_str(&format!("    val h = {}\n", gate("Obj")));
            body.push_str("    val s = jvmToString(h)\n");
            body.push_str("    jvmFree(h)\n");
            body.push_str("    return s\n");
            ("fun", ": String".into(), body)
        }
        RetKind::Obj(simple) => {
            body.push_str(&format!("    return {}({})\n", simple, gate("Obj")));
            ("fun", format!(": {}", simple), body)
        }
    }
}
