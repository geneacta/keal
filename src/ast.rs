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
    Trait(TraitDecl),
    /// `import "./other.keal"` — resolved and inlined by the module loader.
    Import { path: String, span: Span },
    /// A top-level statement, executed in order when the program runs.
    Stmt(Stmt),
}

/// One entry of a `<T, U: Comparable>` list. Bounds are parsed now and
/// enforced once traits exist.
#[derive(Clone, Debug)]
pub struct TypeParam {
    pub name: String,
    pub bounds: Vec<TypeExpr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FunDecl {
    pub name: String,
    pub type_params: Vec<TypeParam>,
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

/// A named set of method signatures a class can promise to provide, and the
/// vocabulary that type-parameter bounds are written in.
#[derive(Clone, Debug)]
pub struct TraitDecl {
    pub name: String,
    pub methods: Vec<TraitMethod>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TraitMethod {
    pub decl: FunDecl,
    /// False when the trait only states the signature and every implementer
    /// must supply the body.
    pub has_default: bool,
}

#[derive(Clone, Debug)]
pub struct ClassDecl {
    pub name: String,
    /// A record is a class whose fields are all immutable and which gets a
    /// field-by-field `equals` for free.
    pub is_record: bool,
    pub type_params: Vec<TypeParam>,
    /// The traits written after `:` in `class Version(...) : Comparable`.
    pub traits: Vec<TypeExpr>,
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
    /// `val Point(x, y) = p` — binds the constructor fields by position.
    Destructure { pattern: Destructuring, init: Expr, mutable: bool },
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
    /// Filled in by the checker. A backend needs to know whether `a + b` is
    /// integer addition, float addition or string concatenation, and this is
    /// where that answer is recorded rather than worked out a second time.
    pub ty: Option<crate::types::Type>,
}

impl Expr {
    /// The type the checker recorded. `None` before checking, or on a node
    /// inside an expression the checker abandoned after an error.
    pub fn ty(&self) -> Option<&crate::types::Type> {
        self.ty.as_ref()
    }
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
    /// A binary logical connective, kept apart from `Binary` because most of
    /// them short-circuit.
    Logical { op: LogicalOp, lhs: Box<Expr>, rhs: Box<Expr> },
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
    /// `is Circle(r) if (r > 10) ->`: an extra condition the arm must also
    /// satisfy, checked after its bindings are in scope.
    pub guard: Option<Expr>,
    pub body: Block,
    pub span: Span,
}

/// `Name(a, _, c)`: the type to match and a name for each constructor field,
/// where `None` stands for a `_` that binds nothing.
#[derive(Clone, Debug)]
pub struct Destructuring {
    pub type_name: String,
    pub binds: Vec<Option<String>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum WhenPattern {
    /// One or more values compared with `==` against the subject, or, for a
    /// subject-less `when`, boolean conditions.
    Values(Vec<Expr>),
    /// `is T`, or `is T(a, b)` which also binds the fields in the arm.
    Is { ty: TypeExpr, negated: bool, binds: Option<Destructuring> },
    In { range: Expr, negated: bool },
    Else,
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

/// The binary logical connectives. `Not` is a unary operator, so it lives in
/// `UnOp` instead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LogicalOp {
    And,
    Or,
    Xor,
    Xnor,
    Nand,
    Nor,
    Implies,
}

impl LogicalOp {
    /// The spelling Keal recommends. `&&`, `||` and `^` are accepted aliases,
    /// but diagnostics always suggest the word.
    pub fn symbol(self) -> &'static str {
        match self {
            LogicalOp::And => "and",
            LogicalOp::Or => "or",
            LogicalOp::Xor => "xor",
            LogicalOp::Xnor => "xnor",
            LogicalOp::Nand => "nand",
            LogicalOp::Nor => "nor",
            LogicalOp::Implies => "implies",
        }
    }

    /// Applies the connective to two known operands.
    pub fn apply(self, a: bool, b: bool) -> bool {
        match self {
            LogicalOp::And => a && b,
            LogicalOp::Or => a || b,
            LogicalOp::Xor => a != b,
            LogicalOp::Xnor => a == b,
            LogicalOp::Nand => !(a && b),
            LogicalOp::Nor => !(a || b),
            LogicalOp::Implies => !a || b,
        }
    }

    /// When the left operand alone settles the result, that result.
    ///
    /// `xor` and `xnor` are absent on purpose: both depend on the right
    /// operand whatever the left one is, so they always evaluate it.
    pub fn short_circuit(self, left: bool) -> Option<bool> {
        match (self, left) {
            (LogicalOp::And, false) => Some(false),
            (LogicalOp::Or, true) => Some(true),
            (LogicalOp::Nand, false) => Some(true),
            (LogicalOp::Nor, true) => Some(false),
            (LogicalOp::Implies, false) => Some(true),
            _ => None,
        }
    }

    /// The truth the left operand must have for the right one to be reached.
    /// This is what lets `x != null implies x.length > 0` type-check.
    pub fn guard(self) -> Option<bool> {
        match self {
            LogicalOp::And | LogicalOp::Nand | LogicalOp::Implies => Some(true),
            LogicalOp::Or | LogicalOp::Nor => Some(false),
            LogicalOp::Xor | LogicalOp::Xnor => None,
        }
    }

    /// When the whole expression is known to be `outcome`, the truth both
    /// operands must then have, if it is forced.
    pub fn implied_operands(self, outcome: bool) -> Option<bool> {
        match (self, outcome) {
            (LogicalOp::And, true) => Some(true),
            (LogicalOp::Or, false) => Some(false),
            (LogicalOp::Nand, false) => Some(true),
            (LogicalOp::Nor, true) => Some(false),
            _ => None,
        }
    }
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
