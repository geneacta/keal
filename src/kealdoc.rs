//! `keal doc` — the `///` comments in, one self-contained HTML page out.
//!
//! The tool parses each file with the real parser (so every signature shown
//! is the one the compiler sees) and pairs declarations with the `///`
//! block that ends on the line above them — the lexer strips comments, so
//! the pairing reads the source text directly, by line. With no files
//! named, it documents the prelude: the standard library reference.
//!
//! One page, no external assets, styled to read well and print well.

use std::collections::HashMap;
use std::process::ExitCode;

use crate::ast::*;
use crate::lexer;
use crate::parser;

pub fn run(args: &[String]) -> ExitCode {
    let mut out_path: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-o" {
            let Some(p) = args.get(i + 1) else {
                eprintln!("error: `-o` needs a file path");
                return ExitCode::FAILURE;
            };
            out_path = Some(p.clone());
            i += 2;
        } else {
            files.push(args[i].clone());
            i += 1;
        }
    }

    let mut modules: Vec<(String, String)> = Vec::new();
    if files.is_empty() {
        modules.push(("the standard library".to_string(), include_str!("prelude.keal").to_string()));
    }
    for f in &files {
        match std::fs::read_to_string(f) {
            Ok(text) => modules.push((f.clone(), text)),
            Err(e) => {
                eprintln!("error: cannot read `{}`: {}", f, e);
                return ExitCode::FAILURE;
            }
        }
    }

    let mut sections = Vec::new();
    for (name, text) in &modules {
        match document(name, text) {
            Ok(s) => sections.push(s),
            Err(msg) => {
                eprintln!("error: `{}` does not parse: {}", name, msg);
                return ExitCode::FAILURE;
            }
        }
    }

    let title = if files.is_empty() {
        "Keal — the standard library".to_string()
    } else if files.len() == 1 {
        format!("Keal — {}", files[0])
    } else {
        format!("Keal — {} modules", files.len())
    };
    let html = page(&title, &sections);
    match out_path {
        Some(p) => match std::fs::write(&p, html) {
            Ok(()) => {
                println!("{}", p);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: cannot write `{}`: {}", p, e);
                ExitCode::FAILURE
            }
        },
        None => {
            print!("{}", html);
            ExitCode::SUCCESS
        }
    }
}

struct Section {
    module: String,
    entries: Vec<Entry>,
}

struct Entry {
    kind: &'static str,
    name: String,
    signature: String,
    doc: String,
    members: Vec<(String, String)>, // (signature, doc)
}

/// The `///` block whose last line is `line - 1`, gathered from the text.
fn docs_by_line(text: &str) -> HashMap<usize, String> {
    let mut out = HashMap::new();
    let mut block: Vec<String> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_start();
        if let Some(rest) = line.strip_prefix("///") {
            block.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        } else {
            if !block.is_empty() && !line.is_empty() {
                out.insert(i + 1, block.join("\n"));
            }
            if !line.is_empty() {
                block.clear();
            }
        }
    }
    out
}

fn document(module: &str, text: &str) -> Result<Section, String> {
    let tokens = lexer::lex(text, 0).map_err(|d| d.msg.clone())?;
    let program = parser::parse(tokens).map_err(|d| d.msg.clone())?;
    let docs = docs_by_line(text);
    let doc_of = |line: usize| docs.get(&line).cloned().unwrap_or_default();

    let mut entries = Vec::new();
    for item in &program.items {
        match item {
            Item::Fun(f) => entries.push(Entry {
                kind: "func",
                name: f.name.clone(),
                signature: fun_sig(f, "func"),
                doc: doc_of(f.span.line as usize),
                members: Vec::new(),
            }),
            Item::Class(c) => {
                let mut members = Vec::new();
                for p in &c.ctor {
                    if let Some(mutable) = p.field {
                        members.push((
                            format!(
                                "{} {}: {}",
                                if mutable { "var" } else { "val" },
                                p.name,
                                ty(&p.ty)
                            ),
                            String::new(),
                        ));
                    }
                }
                for f in &c.fields {
                    let t = f.ty.as_ref().map(ty).unwrap_or_default();
                    members.push((
                        format!("{} {}{}", if f.mutable { "var" } else { "val" }, f.name,
                            if t.is_empty() { String::new() } else { format!(": {}", t) }),
                        doc_of(f.span.line as usize),
                    ));
                }
                for m in &c.methods {
                    members.push((fun_sig(m, if m.ret.is_some() { "func" } else { "proc" }),
                        doc_of(m.span.line as usize)));
                }
                entries.push(Entry {
                    kind: if c.is_record { "record" } else { "class" },
                    name: c.name.clone(),
                    signature: class_sig(c),
                    doc: doc_of(c.span.line as usize),
                    members,
                });
            }
            Item::Trait(t) => {
                let members = t
                    .methods
                    .iter()
                    .map(|m| {
                        let tag = if m.decl.ret.is_some() { "func" } else { "proc" };
                        let suffix = if m.has_default { "  (default)" } else { "" };
                        (format!("{}{}", fun_sig(&m.decl, tag), suffix),
                            doc_of(m.decl.span.line as usize))
                    })
                    .collect();
                entries.push(Entry {
                    kind: "trait",
                    name: t.name.clone(),
                    signature: format!("trait {}", t.name),
                    doc: doc_of(t.span.line as usize),
                    members,
                });
            }
            Item::Extern(x) => entries.push(Entry {
                kind: "extern",
                name: x.name.clone(),
                signature: extern_sig(x),
                doc: doc_of(x.span.line as usize),
                members: Vec::new(),
            }),
            _ => {}
        }
    }
    Ok(Section { module: module.to_string(), entries })
}

fn ty(t: &TypeExpr) -> String {
    match &t.kind {
        TypeExprKind::Named { name, args } => {
            if args.is_empty() {
                name.clone()
            } else {
                format!("{}<{}>", name, args.iter().map(ty).collect::<Vec<_>>().join(", "))
            }
        }
        TypeExprKind::Boundary { mode, inner } => format!("{} {}", mode, ty(inner)),
        TypeExprKind::Nullable(inner) => format!("{}?", ty(inner)),
        TypeExprKind::Fun { params, ret } => format!(
            "({}) -> {}",
            params.iter().map(ty).collect::<Vec<_>>().join(", "),
            ty(ret)
        ),
    }
}

fn type_params(tps: &[TypeParam]) -> String {
    if tps.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = tps
        .iter()
        .map(|p| {
            if p.bounds.is_empty() {
                p.name.clone()
            } else {
                format!(
                    "{}: {}",
                    p.name,
                    p.bounds.iter().map(ty).collect::<Vec<_>>().join(" + ")
                )
            }
        })
        .collect();
    format!("<{}>", parts.join(", "))
}

fn params(ps: &[Param]) -> String {
    ps.iter()
        .map(|p| match &p.ty {
            Some(t) => format!("{}: {}", p.name, ty(t)),
            None => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn fun_sig(f: &FunDecl, tag: &str) -> String {
    let ret = match &f.ret {
        Some(t) => format!(": {}", ty(t)),
        None => String::new(),
    };
    format!("{} {}{}({}){}", tag, f.name, type_params(&f.type_params), params(&f.params), ret)
}

fn class_sig(c: &ClassDecl) -> String {
    let kw = if c.is_record { "record" } else { "class" };
    let ctor = c
        .ctor
        .iter()
        .map(|p| {
            let prefix = match p.field {
                Some(true) => "var ",
                Some(false) => "val ",
                None => "",
            };
            format!("{}{}: {}", prefix, p.name, ty(&p.ty))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let traits = if c.traits.is_empty() {
        String::new()
    } else {
        format!(" : {}", c.traits.iter().map(ty).collect::<Vec<_>>().join(", "))
    };
    format!("{} {}{}({}){}", kw, c.name, type_params(&c.type_params), ctor, traits)
}

fn extern_sig(x: &ExternDecl) -> String {
    let ret = match &x.ret {
        Some(t) => format!(": {}", ty(t)),
        None => String::new(),
    };
    format!("extern func {}({}){}", x.name, params(&x.params), ret)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Doc text with `code` spans honored, paragraphs split on blank lines.
fn doc_html(doc: &str) -> String {
    let mut out = String::new();
    for para in doc.split("\n\n") {
        if para.trim().is_empty() {
            continue;
        }
        let mut html = String::new();
        let mut in_code = false;
        for ch in escape(para.trim()).chars() {
            if ch == '`' {
                html.push_str(if in_code { "</code>" } else { "<code>" });
                in_code = !in_code;
            } else {
                html.push(ch);
            }
        }
        if in_code {
            html.push_str("</code>");
        }
        out.push_str(&format!("<p>{}</p>\n", html));
    }
    out
}

fn slug(module: &str, name: &str) -> String {
    format!("{}-{}", module.replace(['/', '.', ' '], "-"), name)
}

fn page(title: &str, sections: &[Section]) -> String {
    let mut nav = String::new();
    let mut body = String::new();
    for s in sections {
        nav.push_str(&format!("<div class=\"navmod\">{}</div>\n", escape(&s.module)));
        body.push_str(&format!("<h2 id=\"{}\">{}</h2>\n", slug(&s.module, ""), escape(&s.module)));
        for e in &s.entries {
            let id = slug(&s.module, &e.name);
            nav.push_str(&format!(
                "<a href=\"#{}\"><span class=\"k k-{}\">{}</span>{}</a>\n",
                id, e.kind, e.kind, escape(&e.name)
            ));
            body.push_str(&format!(
                "<section id=\"{}\">\n<pre class=\"sig\">{}</pre>\n{}",
                id,
                escape(&e.signature),
                doc_html(&e.doc)
            ));
            if !e.members.is_empty() {
                body.push_str("<div class=\"members\">\n");
                for (sig, doc) in &e.members {
                    body.push_str(&format!("<pre class=\"sig m\">{}</pre>\n{}", escape(sig), doc_html(doc)));
                }
                body.push_str("</div>\n");
            }
            body.push_str("</section>\n");
        }
    }
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ --ink: #1c2330; --dim: #5b6575; --line: #e3e7ee; --bg: #ffffff;
        --side: #f6f8fb; --code: #eef1f6; --accent: #2456c4; }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; font: 16px/1.6 Georgia, 'Times New Roman', serif;
       color: var(--ink); background: var(--bg); }}
.wrap {{ display: flex; min-height: 100vh; }}
nav {{ width: 270px; flex: none; background: var(--side);
      border-right: 1px solid var(--line); padding: 24px 0;
      position: sticky; top: 0; height: 100vh; overflow-y: auto; }}
nav .navmod {{ font-weight: bold; padding: 14px 20px 6px;
              font-variant: small-caps; letter-spacing: .04em; }}
nav a {{ display: block; padding: 3px 20px; color: var(--ink);
        text-decoration: none; font-family: ui-monospace, 'SF Mono', Menlo, monospace;
        font-size: 13px; white-space: nowrap; overflow: hidden;
        text-overflow: ellipsis; }}
nav a:hover {{ background: var(--line); }}
.k {{ display: inline-block; width: 52px; color: var(--dim); font-size: 11px; }}
main {{ flex: 1; max-width: 860px; padding: 36px 48px 96px; }}
h1 {{ font-size: 26px; margin: 0 0 4px; }}
h1 + p {{ color: var(--dim); margin-top: 0; }}
h2 {{ font-size: 15px; font-variant: small-caps; letter-spacing: .06em;
     color: var(--dim); border-bottom: 1px solid var(--line);
     padding-bottom: 6px; margin-top: 44px; }}
section {{ margin: 26px 0; }}
pre.sig {{ font: 14px/1.5 ui-monospace, 'SF Mono', Menlo, monospace;
          background: var(--code); padding: 9px 14px; border-radius: 6px;
          overflow-x: auto; margin: 0 0 8px; }}
pre.sig.m {{ background: none; border-left: 3px solid var(--line);
            border-radius: 0; padding: 2px 14px; margin: 10px 0 4px; }}
.members {{ margin-left: 14px; }}
p {{ margin: 6px 0; }}
code {{ font-family: ui-monospace, 'SF Mono', Menlo, monospace; font-size: 14px;
       background: var(--code); padding: 1px 5px; border-radius: 4px; }}
footer {{ margin-top: 64px; color: var(--dim); font-size: 13px;
         border-top: 1px solid var(--line); padding-top: 12px; }}
@media (max-width: 760px) {{ .wrap {{ display: block; }}
  nav {{ position: static; width: auto; height: auto; }} main {{ padding: 24px; }} }}
</style>
</head>
<body>
<div class="wrap">
<nav>
{nav}</nav>
<main>
<h1>{title}</h1>
<p>Generated by <code>keal doc</code>. Signatures are the compiler's own.</p>
{body}<footer>keal doc</footer>
</main>
</div>
</body>
</html>
"#,
        title = escape(title),
        nav = nav,
        body = body
    )
}
