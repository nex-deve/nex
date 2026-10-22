//! Abstract syntax tree produced by the parser.

use crate::span::Span;

/// Unique id of an expression within one program; keys the type table.
pub type ExprId = usize;

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Use(UseDecl),
    Fn(FnDecl),
    App(AppDecl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UseDecl {
    pub path: Vec<Ident>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeExpr {
    pub kind: TypeExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeExprKind {
    /// `Int`, `Str`, ...
    Named(String),
    /// `[T]`
    Array(Box<TypeExpr>),
    /// `{K: V}`
    Map(Box<TypeExpr>, Box<TypeExpr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeExpr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppDecl {
    pub name: Ident,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    Let {
        mutable: bool,
        name: Ident,
        ty: Option<TypeExpr>,
        value: Expr,
    },
    /// `name = value` or `name[i][j] = value`.
    Assign {
        name: Ident,
        path: Vec<Expr>,
        value: Expr,
    },
    Say(Expr),
    Return(Option<Expr>),
    If {
        cond: Expr,
        then: Block,
        else_: Option<ElseBranch>,
    },
    While {
        cond: Expr,
        body: Block,
    },
    For {
        var: Ident,
        iter: ForIter,
        body: Block,
    },
    /// `try { body } catch err { handler }`; `err` is the error message.
    Try {
        body: Block,
        err: Ident,
        handler: Block,
    },
    /// `fail message`: raise an error with this message.
    Fail(Expr),
    Break,
    Continue,
    Block(Block),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ForIter {
    /// `for i in start..end` (end exclusive)
    Range(Expr, Expr),
    /// `for x in array` or `for k in map` (keys, sorted)
    Each(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ElseBranch {
    Block(Block),
    If(Box<Stmt>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub id: ExprId,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Interp(Vec<InterpPart>),
    Array(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    MethodCall {
        receiver: Box<Expr>,
        method: Ident,
        args: Vec<Expr>,
    },
    Var(String),
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        op_span: Span,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Call {
        callee: Ident,
        args: Vec<Expr>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    And,
    Or,
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
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }
}

/// True if control can never reach the end of `block` (it always returns,
/// breaks or continues).
pub fn block_diverges(block: &Block) -> bool {
    block.stmts.iter().any(stmt_diverges)
}

pub fn stmt_diverges(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue | StmtKind::Fail(_) => true,
        StmtKind::If {
            then,
            else_: Some(else_),
            ..
        } => block_diverges(then) && else_diverges(else_),
        StmtKind::Try { body, handler, .. } => block_diverges(body) && block_diverges(handler),
        StmtKind::Block(b) => block_diverges(b),
        _ => false,
    }
}

fn else_diverges(e: &ElseBranch) -> bool {
    match e {
        ElseBranch::Block(b) => block_diverges(b),
        ElseBranch::If(s) => stmt_diverges(s),
    }
}

/// True if every path through `block` ends in `return` or `fail` (used for
/// the "missing return" check; `break`/`continue` do not count).
pub fn block_returns(block: &Block) -> bool {
    block.stmts.iter().any(stmt_returns)
}

fn stmt_returns(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Fail(_) => true,
        StmtKind::Try { body, handler, .. } => block_returns(body) && block_returns(handler),
        StmtKind::If {
            then,
            else_: Some(else_),
            ..
        } => {
            block_returns(then)
                && match else_ {
                    ElseBranch::Block(b) => block_returns(b),
                    ElseBranch::If(s) => stmt_returns(s),
                }
        }
        StmtKind::Block(b) => block_returns(b),
        _ => false,
    }
}
