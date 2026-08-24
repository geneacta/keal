//! A line-oriented REPL.
//!
//! The checker and the interpreter both persist between inputs, so later
//! entries see earlier declarations. Input keeps being read while brackets
//! are still open, which is what makes multi-line functions and classes work.

use std::io::{BufRead, Write};
use std::process::ExitCode;

use crate::checker::Checker;
use crate::interp::Interp;
use crate::lexer::{self, Tok};
use crate::parser;
use crate::span::Sources;
use crate::types::Type;
use crate::value::Value;

pub fn run() -> ExitCode {
    println!("keal {} — type `:help` for commands, Ctrl-D to leave", crate::VERSION);

    let mut sources = Sources::new();
    let mut checker = Checker::new();
    checker.set_repl(true);
    let mut vm = Interp::new();
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut buffer = String::new();

    loop {
        let prompt = if buffer.is_empty() { ">>> " } else { "... " };
        print!("{}", prompt);
        let _ = std::io::stdout().flush();

        let Some(line) = lines.next() else { break };
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("input error: {}", e);
                break;
            }
        };

        if buffer.is_empty() {
            match line.trim() {
                ":quit" | ":q" => break,
                ":help" | ":h" => {
                    println!(":help  this message");
                    println!(":quit  leave the REPL");
                    println!("Anything else is Keal code. An expression prints its value.");
                    continue;
                }
                "" => continue,
                _ => {}
            }
        }

        buffer.push_str(&line);
        buffer.push('\n');
        if is_incomplete(&buffer) {
            continue;
        }
        let source = std::mem::take(&mut buffer);

        let file = sources.add(format!("<repl:{}>", sources.len()), source.clone());
        let parsed = lexer::lex(&source, file).and_then(parser::parse);
        let mut program = match parsed {
            Ok(p) => p,
            Err(d) => {
                eprint!("{}", sources.render("error", &d));
                continue;
            }
        };

        let (errors, last) = checker.check_program(&mut program);
        if !errors.is_empty() {
            for d in &errors {
                eprint!("{}", sources.render("error", d));
            }
            continue;
        }

        match vm.run_repl(&program) {
            Ok(value) => {
                let prints = !matches!(last, None | Some(Type::Unit) | Some(Type::Never));
                if prints && !matches!(value, Value::Unit) {
                    let span = program
                        .items
                        .last()
                        .map(|_| crate::span::Span::new(file, 1, 1))
                        .unwrap_or_default();
                    match vm.display(&value, span) {
                        Ok(text) => println!("{}", text),
                        Err(_) => println!("<unprintable value>"),
                    }
                }
            }
            Err(e) => {
                eprint!("{}", sources.render("runtime error", &e.diag));
                for (name, span) in e.frames.iter().take(5) {
                    eprintln!("  in `{}` at line {}", name, span.line);
                }
            }
        }
    }

    println!();
    ExitCode::SUCCESS
}

/// True while more input is needed.
///
/// Two things say so: a bracket is still open, or the buffer does not end at
/// a statement boundary. The second falls out of semicolon insertion — the
/// lexer appends a `;` at the final newline exactly when the input could end
/// there, so its absence means the last token was something like `+` or `=`.
fn is_incomplete(buffer: &str) -> bool {
    let Ok(tokens) = lexer::lex(buffer, 0) else {
        // A lexing error is usually an unterminated string, which one more
        // line will not fix; let the parser report it.
        return false;
    };
    let mut depth = 0i32;
    for t in &tokens {
        match t.tok {
            Tok::LParen | Tok::LBrace | Tok::LBracket => depth += 1,
            Tok::RParen | Tok::RBrace | Tok::RBracket => depth -= 1,
            _ => {}
        }
    }
    if depth > 0 {
        return true;
    }
    match tokens.iter().rev().find(|t| t.tok != Tok::Eof) {
        // Nothing but whitespace and comments: there is nothing to wait for.
        None => false,
        Some(last) => !last.tok.ends_statement() && last.tok != Tok::Semi,
    }
}
