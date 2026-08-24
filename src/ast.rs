//! The abstract syntax tree produced by the parser.

use crate::span::Span;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct Program {
    pub items: Vec<Item>,
}

/// Top-level declarations. Statements are only legal inside function bodies,
/// except that a script's top level also collects them into `main`-like code.
#[derive(Clone, Debug)]
pub enum Item {
    Fun(FunDecl),
    Class(ClassDecl),
    /// `import "./other.keal"` — resolved and inlined by the module loader.
    Import { path: String, span: Span },
    /// A top-level statement, executed in order when the program runs.
    Stmt(Stmt),
}

#[derive(Clone, Debug)]
pub struct FunDecl {
    pub name: String,
    pub params: Rc<Vec<Param>>,
    pub ret: Option<TypeExpr>,
    pub body: Rc<Block>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: Option<TypeExpr>,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ClassDecl {
    pub name: String,
    /// Primary-constructor parameters. Those marked `val`/`var` become fields.
    pub ctor: Vec<CtorParam>,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<FunDecl>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct CtorParam {
    pub name: String,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
    /// `Some(true)` for `var`, `Some(false)` for `val`, `None` for a plain param.
    pub field: Option<bool>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub name: String,
    pub ty: Option<TypeExpr>,
    pub init: Option<Expr>,
    pub mutable: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    /// `val`/`var` binding. `mutable` distinguishes them.
    Let { name: String, ty: Option<TypeExpr>, init: Expr, mutable: bool },
    Expr(Expr),
    Return(Option<Expr>),
    While { cond: Expr, body: Block },
    For { var: String, ty: Option<TypeExpr>, iter: Expr, body: Block },
    Break,
    Continue,
    /// A nested function declaration.
    Fun(FunDecl),
    Class(ClassDecl),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    /// String literal with `${...}` holes, already parsed.
    Interp(Vec<InterpPart>),
    Null,
    This,
    Ident(String),

    Unary { op: UnOp, rhs: Box<Expr> },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    /// `&&` / `||`, kept separate because they short-circuit.
    Logical { or: bool, lhs: Box<Expr>, rhs: Box<Expr> },
    /// `a ?: b` — evaluates `b` only when `a` is null.
    Elvis { lhs: Box<Expr>, rhs: Box<Expr> },
    Assign { target: Box<Expr>, op: Option<BinOp>, value: Box<Expr> },

    Call { callee: Box<Expr>, args: Vec<Arg> },
    /// `obj.name`, or `obj?.name` when `safe`.
    Field { obj: Box<Expr>, name: String, safe: bool },
    /// `obj.name(args)`, kept distinct so built-in methods can be typed.
    MethodCall { obj: Box<Expr>, name: String, args: Vec<Arg>, safe: bool },
    Index { obj: Box<Expr>, index: Box<Expr> },

    If { cond: Box<Expr>, then: Block, els: Option<Box<Else>> },
    When { subject: Option<Box<Expr>>, arms: Vec<WhenArm> },
    ListLit(Vec<Expr>),
    MapLit(Vec<(Expr, Expr)>),
    Lambda { params: Rc<Vec<Param>>, body: Rc<Block> },
    Range { start: Box<Expr>, end: Box<Expr> },
    /// `value!!` — narrows `T?` to `T`, panicking at run time if it is null.
    NotNull(Box<Expr>),
    Is { value: Box<Expr>, ty: TypeExpr, negated: bool },
}

#[derive(Clone, Debug)]
pub struct Arg {
    /// `Some` for a named argument `name = value`.
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub enum Else {
    Block(Block),
    If(Expr),
}

#[derive(Clone, Debug)]
pub struct WhenArm {
    pub pattern: WhenPattern,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum WhenPattern {
    /// One or more values compared with `==` against the subject, or, for a
    /// subject-less `when`, boolean conditions.
    Values(Vec<Expr>),
    Is { ty: TypeExpr, negated: bool },
    In { range: Expr, negated: bool },
    Else,
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
        }
    }

    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

/// Syntax of a type as written by the user, resolved to a `Type` by the checker.
#[derive(Clone, Debug)]
pub struct TypeExpr {
    pub kind: TypeExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeExprKind {
    /// A name plus optional type arguments: `Int`, `List<String>`.
    Named { name: String, args: Vec<TypeExpr> },
    /// `T?`
    Nullable(Box<TypeExpr>),
    /// `(A, B) -> C`
    Fun { params: Vec<TypeExpr>, ret: Box<TypeExpr> },
}
