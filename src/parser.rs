//! Recursive-descent parser with precedence climbing for binary operators.
//!
//! Braces are mandatory on `if`/`while`/`for` bodies, which keeps `{` free to
//! mean "lambda" in every expression position.

use crate::ast::*;
use crate::lexer::{self, StrPart, Tok, Token};
use crate::span::{Diag, Span};
use std::rc::Rc;

pub fn parse(tokens: Vec<Token>) -> Result<Program, Diag> {
    Parser { toks: tokens, pos: 0 }.program()
}

/// Parses a single expression, used for `${...}` holes and the REPL.
pub fn parse_expr_only(tokens: Vec<Token>) -> Result<Expr, Diag> {
    let mut p = Parser { toks: tokens, pos: 0 };
    let e = p.expr()?;
    p.skip_semis();
    if !p.at(&Tok::Eof) {
        return Err(p.err_here("unexpected trailing input in expression"));
    }
    Ok(e)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

/// Binding powers, lowest first. Assignment is handled at statement level.
fn binary_power(tok: &Tok) -> Option<(u8, BinOp)> {
    Some(match tok {
        Tok::EqEq => (4, BinOp::Eq),
        Tok::BangEq => (4, BinOp::Ne),
        Tok::Lt => (5, BinOp::Lt),
        Tok::LtEq => (5, BinOp::Le),
        Tok::Gt => (5, BinOp::Gt),
        Tok::GtEq => (5, BinOp::Ge),
        Tok::Plus => (8, BinOp::Add),
        Tok::Minus => (8, BinOp::Sub),
        Tok::Star => (9, BinOp::Mul),
        Tok::Slash => (9, BinOp::Div),
        Tok::Percent => (9, BinOp::Rem),
        _ => return None,
    })
}

/// `xor`, `xnor`, `nand`, `nor` and `implies` are contextual: they are only
/// read as operators where a binary operator may appear, so a variable may
/// still be called `nor`. Semicolon insertion keeps that safe across lines.
fn word_op(tok: &Tok) -> Option<LogicalOp> {
    let Tok::Ident(name) = tok else { return None };
    Some(match name.as_str() {
        "xor" => LogicalOp::Xor,
        "xnor" => LogicalOp::Xnor,
        "nand" => LogicalOp::Nand,
        "nor" => LogicalOp::Nor,
        "implies" => LogicalOp::Implies,
        _ => return None,
    })
}

/// The word operators all sit at one level, below `&&` and `||`.
const P_WORD: u8 = 1;
const P_OR: u8 = 2;
const P_AND: u8 = 3;
const P_CMP: u8 = 5;
const P_ELVIS: u8 = 6;
const P_RANGE: u8 = 7;
const P_UNARY: u8 = 10;

impl Parser {
    // ---- token helpers -------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos.min(self.toks.len() - 1)].span
    }

    fn at(&self, t: &Tok) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(t)
    }

    fn advance(&mut self) -> Token {
        let t = self.toks[self.pos.min(self.toks.len() - 1)].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: Tok, ctx: &str) -> Result<Token, Diag> {
        if self.at(&t) {
            Ok(self.advance())
        } else {
            Err(Diag::new(
                self.span(),
                format!("expected {} {}, found {}", t.describe(), ctx, self.peek().describe()),
            ))
        }
    }

    fn expect_ident(&mut self, ctx: &str) -> Result<(String, Span), Diag> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Ident(name) => {
                self.advance();
                Ok((name, span))
            }
            other => Err(Diag::new(
                span,
                format!("expected {}, found {}", ctx, other.describe()),
            )),
        }
    }

    fn err_here(&self, msg: impl Into<String>) -> Diag {
        Diag::new(self.span(), msg)
    }

    fn skip_semis(&mut self) {
        while self.at(&Tok::Semi) {
            self.advance();
        }
    }

    // ---- top level -----------------------------------------------------

    fn program(mut self) -> Result<Program, Diag> {
        let mut items = Vec::new();
        loop {
            self.skip_semis();
            if self.at(&Tok::Eof) {
                break;
            }
            items.push(self.item()?);
        }
        Ok(Program { items })
    }

    fn item(&mut self) -> Result<Item, Diag> {
        match self.peek() {
            Tok::Fun => Ok(Item::Fun(self.fun_decl()?)),
            Tok::Class => Ok(Item::Class(self.class_decl()?)),
            Tok::Ident(name) if name == "trait" => Ok(Item::Trait(self.trait_decl()?)),
            Tok::Import => {
                let span = self.span();
                self.advance();
                match self.peek().clone() {
                    Tok::Str(parts) => {
                        self.advance();
                        match parts.as_slice() {
                            [StrPart::Lit(path)] => Ok(Item::Import { path: path.clone(), span }),
                            _ => Err(Diag::new(span, "import path must be a plain string literal")),
                        }
                    }
                    other => Err(Diag::new(
                        self.span(),
                        format!("expected an import path string, found {}", other.describe()),
                    )),
                }
            }
            _ => Ok(Item::Stmt(self.stmt()?)),
        }
    }

    fn fun_decl(&mut self) -> Result<FunDecl, Diag> {
        let span = self.span();
        self.expect(Tok::Fun, "to start a function declaration")?;
        let (name, _) = self.expect_ident("a function name")?;
        let type_params = self.type_param_list()?;
        let params = self.param_list()?;
        let ret = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
        let body = self.block()?;
        Ok(FunDecl {
            name,
            type_params,
            params: Rc::new(params),
            ret,
            body: Rc::new(body),
            span,
        })
    }

    /// An optional `<T, U: Bound, V>` after a `fun` or `class` name.
    fn type_param_list(&mut self) -> Result<Vec<TypeParam>, Diag> {
        let mut out = Vec::new();
        if !self.at(&Tok::Lt) {
            return Ok(out);
        }
        self.advance();
        while !self.at(&Tok::Gt) {
            let (name, span) = self.expect_ident("a type parameter name")?;
            let mut bounds = Vec::new();
            if self.eat(&Tok::Colon) {
                loop {
                    bounds.push(self.type_expr()?);
                    // `T: A + B` requires every bound at once.
                    if !self.eat(&Tok::Plus) {
                        break;
                    }
                }
            }
            out.push(TypeParam { name, bounds, span });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::Gt, "to close the type parameter list")?;
        if out.is_empty() {
            return Err(self.err_here("a type parameter list cannot be empty"));
        }
        Ok(out)
    }

    fn param_list(&mut self) -> Result<Vec<Param>, Diag> {
        self.expect(Tok::LParen, "to start a parameter list")?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            let (name, span) = self.expect_ident("a parameter name")?;
            self.expect(Tok::Colon, "after a parameter name")?;
            let ty = Some(self.type_expr()?);
            let default = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
            params.push(Param { name, ty, default, span });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "to close the parameter list")?;
        Ok(params)
    }

    /// `trait Name { fun m(...): T ; fun d(...): T { default body } }`
    ///
    /// `trait` is a contextual keyword: it only introduces a declaration at
    /// the start of an item, so existing code using it as a name still works.
    fn trait_decl(&mut self) -> Result<TraitDecl, Diag> {
        let span = self.span();
        self.advance();
        let (name, _) = self.expect_ident("a trait name")?;
        self.expect(Tok::LBrace, "to open the trait body")?;
        let mut methods = Vec::new();
        loop {
            self.skip_semis();
            if self.at(&Tok::RBrace) || self.at(&Tok::Eof) {
                break;
            }
            let mspan = self.span();
            self.expect(Tok::Fun, "to start a trait method")?;
            let (mname, _) = self.expect_ident("a method name")?;
            let type_params = self.type_param_list()?;
            let params = self.param_list()?;
            let ret = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
            let has_default = self.at(&Tok::LBrace);
            let body = if has_default { self.block()? } else { Block { stmts: Vec::new() } };
            methods.push(TraitMethod {
                decl: FunDecl {
                    name: mname,
                    type_params,
                    params: Rc::new(params),
                    ret,
                    body: Rc::new(body),
                    span: mspan,
                },
                has_default,
            });
        }
        self.expect(Tok::RBrace, "to close the trait body")?;
        Ok(TraitDecl { name, methods, span })
    }

    fn class_decl(&mut self) -> Result<ClassDecl, Diag> {
        let span = self.span();
        self.expect(Tok::Class, "to start a class declaration")?;
        let (name, _) = self.expect_ident("a class name")?;
        let type_params = self.type_param_list()?;

        let mut ctor = Vec::new();
        if self.at(&Tok::LParen) {
            self.advance();
            while !self.at(&Tok::RParen) {
                let pspan = self.span();
                let field = if self.eat(&Tok::Val) {
                    Some(false)
                } else if self.eat(&Tok::Var) {
                    Some(true)
                } else {
                    None
                };
                let (pname, _) = self.expect_ident("a constructor parameter name")?;
                self.expect(Tok::Colon, "after a constructor parameter name")?;
                let ty = self.type_expr()?;
                let default = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
                ctor.push(CtorParam { name: pname, ty, default, field, span: pspan });
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::RParen, "to close the constructor parameter list")?;
        }

        let mut traits = Vec::new();
        if self.eat(&Tok::Colon) {
            loop {
                traits.push(self.type_expr()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        // A class whose state is entirely in its constructor needs no body.
        if !self.at(&Tok::LBrace) {
            return Ok(ClassDecl { name, type_params, traits, ctor, fields, methods, span });
        }
        self.expect(Tok::LBrace, "to open the class body")?;
        loop {
            self.skip_semis();
            if self.at(&Tok::RBrace) || self.at(&Tok::Eof) {
                break;
            }
            match self.peek() {
                Tok::Fun => methods.push(self.fun_decl()?),
                Tok::Val | Tok::Var => {
                    let fspan = self.span();
                    let mutable = matches!(self.advance().tok, Tok::Var);
                    let (fname, _) = self.expect_ident("a field name")?;
                    let ty = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
                    let init = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
                    if ty.is_none() && init.is_none() {
                        return Err(Diag::new(
                            fspan,
                            format!("field `{}` needs either a type or an initializer", fname),
                        ));
                    }
                    fields.push(FieldDecl { name: fname, ty, init, mutable, span: fspan });
                }
                other => {
                    return Err(Diag::new(
                        self.span(),
                        format!(
                            "expected `fun`, `val` or `var` in a class body, found {}",
                            other.describe()
                        ),
                    ))
                }
            }
        }
        self.expect(Tok::RBrace, "to close the class body")?;
        Ok(ClassDecl { name, type_params, traits, ctor, fields, methods, span })
    }

    // ---- statements ----------------------------------------------------

    fn block(&mut self) -> Result<Block, Diag> {
        self.expect(Tok::LBrace, "to open a block")?;
        let stmts = self.stmts_until_brace()?;
        self.expect(Tok::RBrace, "to close the block")?;
        Ok(Block { stmts })
    }

    fn stmts_until_brace(&mut self) -> Result<Vec<Stmt>, Diag> {
        let mut stmts = Vec::new();
        loop {
            self.skip_semis();
            if self.at(&Tok::RBrace) || self.at(&Tok::Eof) {
                break;
            }
            stmts.push(self.stmt()?);
        }
        Ok(stmts)
    }

    fn stmt(&mut self) -> Result<Stmt, Diag> {
        let span = self.span();
        let kind = match self.peek() {
            Tok::Val | Tok::Var => {
                let mutable = matches!(self.advance().tok, Tok::Var);
                let (name, _) = self.expect_ident("a variable name")?;
                let ty = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
                self.expect(Tok::Assign, "in a variable declaration")?;
                let init = self.expr()?;
                StmtKind::Let { name, ty, init, mutable }
            }
            Tok::Return => {
                self.advance();
                let value = if self.at(&Tok::Semi) || self.at(&Tok::RBrace) || self.at(&Tok::Eof) {
                    None
                } else {
                    Some(self.expr()?)
                };
                StmtKind::Return(value)
            }
            Tok::Break => {
                self.advance();
                StmtKind::Break
            }
            Tok::Continue => {
                self.advance();
                StmtKind::Continue
            }
            Tok::While => {
                self.advance();
                self.expect(Tok::LParen, "after `while`")?;
                let cond = self.expr()?;
                self.expect(Tok::RParen, "after the `while` condition")?;
                let body = self.block()?;
                StmtKind::While { cond, body }
            }
            Tok::For => {
                self.advance();
                self.expect(Tok::LParen, "after `for`")?;
                let (var, _) = self.expect_ident("a loop variable name")?;
                let ty = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
                self.expect(Tok::In, "after the loop variable")?;
                let iter = self.expr()?;
                self.expect(Tok::RParen, "after the `for` iterable")?;
                let body = self.block()?;
                StmtKind::For { var, ty, iter, body }
            }
            Tok::Fun => StmtKind::Fun(self.fun_decl()?),
            Tok::Class => StmtKind::Class(self.class_decl()?),
            _ => {
                let target = self.expr()?;
                let op = match self.peek() {
                    Tok::Assign => None,
                    Tok::PlusEq => Some(BinOp::Add),
                    Tok::MinusEq => Some(BinOp::Sub),
                    Tok::StarEq => Some(BinOp::Mul),
                    Tok::SlashEq => Some(BinOp::Div),
                    Tok::PercentEq => Some(BinOp::Rem),
                    _ => return Ok(Stmt { kind: StmtKind::Expr(target), span }),
                };
                let op_span = self.span();
                self.advance();
                if !matches!(
                    target.kind,
                    ExprKind::Ident(_) | ExprKind::Field { .. } | ExprKind::Index { .. }
                ) {
                    return Err(Diag::new(op_span, "left side of an assignment is not assignable")
                        .with_note("only variables, fields and indexed elements can be assigned"));
                }
                let value = self.expr()?;
                StmtKind::Expr(Expr {
                    span,
                    kind: ExprKind::Assign {
                        target: Box::new(target),
                        op,
                        value: Box::new(value),
                    },
                })
            }
        };
        Ok(Stmt { kind, span })
    }

    // ---- expressions ---------------------------------------------------

    fn expr(&mut self) -> Result<Expr, Diag> {
        self.binary(0)
    }

    fn binary(&mut self, min_bp: u8) -> Result<Expr, Diag> {
        let mut lhs = self.unary()?;
        loop {
            let span = self.span();
            // Logical, elvis, range and `is`/`in` are infix operators that do
            // not map onto BinOp, so they get their own arms.
            if let Some(op) = word_op(self.peek()) {
                if P_WORD < min_bp {
                    break;
                }
                self.advance();
                // `implies` chains to the right: `a implies b implies c`
                // reads as `a implies (b implies c)`, as in logic.
                let rhs_bp = if op == LogicalOp::Implies { P_WORD } else { P_WORD + 1 };
                let rhs = self.binary(rhs_bp)?;
                lhs = Expr {
                    span,
                    kind: ExprKind::Logical { op, lhs: Box::new(lhs), rhs: Box::new(rhs) },
                };
                // Two different connectives in a row read ambiguously, and
                // `nand`/`nor` are not associative even with themselves.
                if let Some(next) = word_op(self.peek()) {
                    let chainable = next == op && matches!(op, LogicalOp::Xor | LogicalOp::Xnor);
                    if !chainable {
                        let what = if next == op {
                            format!("`{}` does not chain", op.symbol())
                        } else {
                            format!(
                                "`{}` and `{}` cannot be combined without parentheses",
                                op.symbol(),
                                next.symbol()
                            )
                        };
                        return Err(Diag::new(self.span(), what).with_note(format!(
                            "group the operands explicitly, e.g. `(a {} b) {} c`",
                            op.symbol(),
                            next.symbol()
                        )));
                    }
                }
                continue;
            }

            match self.peek() {
                Tok::OrOr if P_OR >= min_bp => {
                    self.advance();
                    let rhs = self.binary(P_OR + 1)?;
                    lhs = Expr {
                        span,
                        kind: ExprKind::Logical {
                            op: LogicalOp::Or,
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                        },
                    };
                    continue;
                }
                Tok::AndAnd if P_AND >= min_bp => {
                    self.advance();
                    let rhs = self.binary(P_AND + 1)?;
                    lhs = Expr {
                        span,
                        kind: ExprKind::Logical {
                            op: LogicalOp::And,
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                        },
                    };
                    continue;
                }
                Tok::Elvis if P_ELVIS >= min_bp => {
                    self.advance();
                    // Right-associative: `a ?: b ?: c` is `a ?: (b ?: c)`.
                    let rhs = self.binary(P_ELVIS)?;
                    lhs = Expr {
                        span,
                        kind: ExprKind::Elvis { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                    };
                    continue;
                }
                Tok::DotDot if P_RANGE >= min_bp => {
                    self.advance();
                    let rhs = self.binary(P_RANGE + 1)?;
                    lhs = Expr {
                        span,
                        kind: ExprKind::Range { start: Box::new(lhs), end: Box::new(rhs) },
                    };
                    continue;
                }
                Tok::Is if P_CMP >= min_bp => {
                    self.advance();
                    let ty = self.type_expr()?;
                    lhs = Expr {
                        span,
                        kind: ExprKind::Is { value: Box::new(lhs), ty, negated: false },
                    };
                    continue;
                }
                Tok::Bang if matches!(self.peek_at(1), Tok::Is | Tok::In) && P_CMP >= min_bp => {
                    self.advance();
                    if self.eat(&Tok::Is) {
                        let ty = self.type_expr()?;
                        lhs = Expr {
                            span,
                            kind: ExprKind::Is { value: Box::new(lhs), ty, negated: true },
                        };
                    } else {
                        self.advance(); // `in`
                        let rhs = self.binary(P_CMP + 1)?;
                        lhs = Expr {
                            span,
                            kind: ExprKind::Unary {
                                op: UnOp::Not,
                                rhs: Box::new(Expr {
                                    span,
                                    kind: ExprKind::MethodCall {
                                        obj: Box::new(rhs),
                                        name: "contains".into(),
                                        args: vec![Arg { name: None, value: lhs }],
                                        safe: false,
                                    },
                                }),
                            },
                        };
                    }
                    continue;
                }
                Tok::In if P_CMP >= min_bp => {
                    self.advance();
                    let rhs = self.binary(P_CMP + 1)?;
                    // `x in xs` desugars to `xs.contains(x)`.
                    lhs = Expr {
                        span,
                        kind: ExprKind::MethodCall {
                            obj: Box::new(rhs),
                            name: "contains".into(),
                            args: vec![Arg { name: None, value: lhs }],
                            safe: false,
                        },
                    };
                    continue;
                }
                _ => {}
            }

            let Some((bp, op)) = binary_power(self.peek()) else { break };
            if bp < min_bp {
                break;
            }
            self.advance();
            let rhs = self.binary(bp + 1)?;
            lhs = Expr {
                span,
                kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) },
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, Diag> {
        let span = self.span();
        let op = match self.peek() {
            Tok::Minus => UnOp::Neg,
            Tok::Bang => UnOp::Not,
            _ => return self.postfix(),
        };
        self.advance();
        let rhs = self.binary(P_UNARY)?;
        Ok(Expr { span, kind: ExprKind::Unary { op, rhs: Box::new(rhs) } })
    }

    fn postfix(&mut self) -> Result<Expr, Diag> {
        // Errors about a call or an index read best when they point at the
        // start of the whole expression rather than at the bracket.
        let start = self.span();
        let mut e = self.primary()?;
        loop {
            let span = start;
            match self.peek() {
                Tok::Dot | Tok::SafeDot => {
                    let safe = matches!(self.advance().tok, Tok::SafeDot);
                    // Member errors point at the member name itself.
                    let (name, span) = self.expect_ident("a field or method name")?;
                    if self.at(&Tok::LParen) {
                        let args = self.arg_list()?;
                        e = Expr {
                            span,
                            kind: ExprKind::MethodCall { obj: Box::new(e), name, args, safe },
                        };
                    } else {
                        e = Expr { span, kind: ExprKind::Field { obj: Box::new(e), name, safe } };
                    }
                }
                Tok::LParen => {
                    let args = self.arg_list()?;
                    e = Expr { span, kind: ExprKind::Call { callee: Box::new(e), args } };
                }
                Tok::BangBang => {
                    self.advance();
                    e = Expr { span, kind: ExprKind::NotNull(Box::new(e)) };
                }
                Tok::LBracket => {
                    self.advance();
                    let index = self.expr()?;
                    self.expect(Tok::RBracket, "to close an index expression")?;
                    e = Expr {
                        span,
                        kind: ExprKind::Index { obj: Box::new(e), index: Box::new(index) },
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn arg_list(&mut self) -> Result<Vec<Arg>, Diag> {
        self.expect(Tok::LParen, "to start an argument list")?;
        let mut args = Vec::new();
        while !self.at(&Tok::RParen) {
            // `name = value` is a named argument; `name == value` is not.
            let name = match (self.peek(), self.peek_at(1)) {
                (Tok::Ident(n), Tok::Assign) => {
                    let n = n.clone();
                    self.advance();
                    self.advance();
                    Some(n)
                }
                _ => None,
            };
            let value = self.expr()?;
            args.push(Arg { name, value });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "to close the argument list")?;
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr, Diag> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Int(n) => {
                self.advance();
                ExprKind::Int(n)
            }
            Tok::Float(n) => {
                self.advance();
                ExprKind::Float(n)
            }
            Tok::True => {
                self.advance();
                ExprKind::Bool(true)
            }
            Tok::False => {
                self.advance();
                ExprKind::Bool(false)
            }
            Tok::Null => {
                self.advance();
                ExprKind::Null
            }
            Tok::This => {
                self.advance();
                ExprKind::This
            }
            Tok::Ident(name) => {
                self.advance();
                ExprKind::Ident(name)
            }
            Tok::Str(parts) => {
                self.advance();
                self.string_expr(parts)?
            }
            Tok::LParen => {
                self.advance();
                let inner = self.expr()?;
                self.expect(Tok::RParen, "to close a parenthesised expression")?;
                return Ok(inner);
            }
            Tok::LBracket => {
                self.advance();
                let mut items = Vec::new();
                while !self.at(&Tok::RBracket) {
                    self.skip_semis();
                    if self.at(&Tok::RBracket) {
                        break;
                    }
                    items.push(self.expr()?);
                    self.skip_semis();
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.skip_semis();
                self.expect(Tok::RBracket, "to close a list literal")?;
                ExprKind::ListLit(items)
            }
            Tok::If => return self.if_expr(),
            Tok::When => return self.when_expr(),
            Tok::LBrace => return self.brace_expr(),
            other => {
                return Err(Diag::new(
                    span,
                    format!("expected an expression, found {}", other.describe()),
                ))
            }
        };
        Ok(Expr { span, kind })
    }

    fn string_expr(&mut self, parts: Vec<StrPart>) -> Result<ExprKind, Diag> {
        if let [StrPart::Lit(s)] = parts.as_slice() {
            return Ok(ExprKind::Str(s.clone()));
        }
        let mut out = Vec::new();
        for part in parts {
            match part {
                StrPart::Lit(s) => out.push(InterpPart::Lit(s)),
                StrPart::Interp(src, span) => {
                    let toks = lexer::lex_fragment(&src, span)?;
                    let e = parse_expr_only(toks)?;
                    out.push(InterpPart::Expr(e));
                }
            }
        }
        Ok(ExprKind::Interp(out))
    }

    fn if_expr(&mut self) -> Result<Expr, Diag> {
        let span = self.span();
        self.expect(Tok::If, "to start an `if`")?;
        self.expect(Tok::LParen, "after `if`")?;
        let cond = self.expr()?;
        self.expect(Tok::RParen, "after the `if` condition")?;
        let then = self.block()?;

        // A newline between `}` and `else` inserted a virtual `;`; skip it,
        // but only if `else` really follows.
        let save = self.pos;
        self.skip_semis();
        let els = if self.eat(&Tok::Else) {
            if self.at(&Tok::If) {
                Some(Box::new(Else::If(self.if_expr()?)))
            } else {
                Some(Box::new(Else::Block(self.block()?)))
            }
        } else {
            self.pos = save;
            None
        };
        Ok(Expr { span, kind: ExprKind::If { cond: Box::new(cond), then, els } })
    }

    fn when_expr(&mut self) -> Result<Expr, Diag> {
        let span = self.span();
        self.expect(Tok::When, "to start a `when`")?;
        let subject = if self.eat(&Tok::LParen) {
            let e = self.expr()?;
            self.expect(Tok::RParen, "after the `when` subject")?;
            Some(Box::new(e))
        } else {
            None
        };
        self.expect(Tok::LBrace, "to open a `when` body")?;
        let mut arms = Vec::new();
        loop {
            self.skip_semis();
            if self.at(&Tok::RBrace) || self.at(&Tok::Eof) {
                break;
            }
            let arm_span = self.span();
            let pattern = if self.eat(&Tok::Else) {
                WhenPattern::Else
            } else if self.eat(&Tok::Is) {
                WhenPattern::Is { ty: self.type_expr()?, negated: false }
            } else if self.at(&Tok::Bang) && matches!(self.peek_at(1), Tok::Is | Tok::In) {
                self.advance();
                if self.eat(&Tok::Is) {
                    WhenPattern::Is { ty: self.type_expr()?, negated: true }
                } else {
                    self.advance();
                    WhenPattern::In { range: self.expr()?, negated: true }
                }
            } else if self.eat(&Tok::In) {
                WhenPattern::In { range: self.expr()?, negated: false }
            } else {
                let mut values = vec![self.expr()?];
                while self.eat(&Tok::Comma) {
                    values.push(self.expr()?);
                }
                WhenPattern::Values(values)
            };
            self.expect(Tok::Arrow, "after a `when` pattern")?;
            // Inside a `when` arm `{` means a block, not a lambda.
            let body = if self.at(&Tok::LBrace) {
                self.block()?
            } else {
                let e = self.expr()?;
                let s = e.span;
                Block { stmts: vec![Stmt { kind: StmtKind::Expr(e), span: s }] }
            };
            arms.push(WhenArm { pattern, body, span: arm_span });
        }
        self.expect(Tok::RBrace, "to close the `when` body")?;
        Ok(Expr { span, kind: ExprKind::When { subject, arms } })
    }

    /// A `{...}` in expression position is one of three things, disambiguated
    /// in this order: a lambda with an explicit parameter list (`{ x -> .. }`),
    /// a map literal (`{}` or `{ k: v, .. }`), or a lambda whose single
    /// parameter is the implicit `it`.
    fn brace_expr(&mut self) -> Result<Expr, Diag> {
        let span = self.span();
        self.expect(Tok::LBrace, "to open a lambda or map literal")?;
        let save = self.pos;

        if let Some(params) = self.lambda_params() {
            return self.lambda_body(span, params);
        }
        self.pos = save;

        if let Some(map) = self.try_map_literal(span)? {
            return Ok(map);
        }
        self.pos = save;

        let params = vec![Param { name: "it".into(), ty: None, default: None, span }];
        self.lambda_body(span, params)
    }

    /// `{}` and `{ key: value, ... }`. Returns `None` if the braces do not
    /// hold a map literal, leaving the caller to restore the position.
    fn try_map_literal(&mut self, span: Span) -> Result<Option<Expr>, Diag> {
        self.skip_semis();
        if self.eat(&Tok::RBrace) {
            return Ok(Some(Expr { span, kind: ExprKind::MapLit(Vec::new()) }));
        }
        let Ok(first_key) = self.expr() else { return Ok(None) };
        if !self.at(&Tok::Colon) {
            return Ok(None);
        }
        self.advance();
        let mut entries = vec![(first_key, self.expr()?)];
        while self.eat(&Tok::Comma) {
            self.skip_semis();
            if self.at(&Tok::RBrace) {
                break;
            }
            let key = self.expr()?;
            self.expect(Tok::Colon, "between a map key and its value")?;
            entries.push((key, self.expr()?));
        }
        self.skip_semis();
        self.expect(Tok::RBrace, "to close a map literal")?;
        Ok(Some(Expr { span, kind: ExprKind::MapLit(entries) }))
    }

    fn lambda_body(&mut self, span: Span, params: Vec<Param>) -> Result<Expr, Diag> {
        let stmts = self.stmts_until_brace()?;
        self.expect(Tok::RBrace, "to close the lambda")?;
        Ok(Expr {
            span,
            kind: ExprKind::Lambda { params: Rc::new(params), body: Rc::new(Block { stmts }) },
        })
    }

    /// Tries to consume `x: T, y ->`. Returns `None` (leaving `self.pos`
    /// wherever it got to; the caller restores it) if this isn't a param list.
    fn lambda_params(&mut self) -> Option<Vec<Param>> {
        let mut params = Vec::new();
        if self.at(&Tok::Arrow) {
            self.advance();
            return Some(params);
        }
        loop {
            let span = self.span();
            let Tok::Ident(name) = self.peek().clone() else { return None };
            self.advance();
            let ty = if self.eat(&Tok::Colon) { Some(self.type_expr().ok()?) } else { None };
            params.push(Param { name, ty, default: None, span });
            if self.eat(&Tok::Comma) {
                continue;
            }
            return if self.eat(&Tok::Arrow) { Some(params) } else { None };
        }
    }

    // ---- types ---------------------------------------------------------

    fn type_expr(&mut self) -> Result<TypeExpr, Diag> {
        let span = self.span();
        let mut ty = match self.peek().clone() {
            Tok::Ident(name) => {
                self.advance();
                let mut args = Vec::new();
                if self.at(&Tok::Lt) {
                    self.advance();
                    loop {
                        args.push(self.type_expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::Gt, "to close type arguments")?;
                }
                TypeExpr { span, kind: TypeExprKind::Named { name, args } }
            }
            Tok::LParen => {
                self.advance();
                let mut params = Vec::new();
                while !self.at(&Tok::RParen) {
                    params.push(self.type_expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RParen, "to close a function type's parameters")?;
                self.expect(Tok::Arrow, "in a function type")?;
                let ret = Box::new(self.type_expr()?);
                TypeExpr { span, kind: TypeExprKind::Fun { params, ret } }
            }
            other => {
                return Err(Diag::new(
                    span,
                    format!("expected a type, found {}", other.describe()),
                ))
            }
        };
        while self.eat(&Tok::Question) {
            ty = TypeExpr { span, kind: TypeExprKind::Nullable(Box::new(ty)) };
        }
        Ok(ty)
    }
}
