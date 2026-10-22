//! Semantic analysis: name resolution and type checking in one pass.
//!
//! Collects all errors it can find instead of stopping at the first one.
//! Expressions whose type cannot be determined are given no type, which
//! suppresses follow-on errors that would only repeat the original problem.
//!
//! Checking is bidirectional where it matters: an expected type flows into
//! empty `[]`/`{}` literals so that `let xs: [Int] = []` works.

use crate::ast::*;
use crate::bytecode::{Builtin, Method};
use crate::diag::Diagnostic;
use crate::project::Unit;
use crate::span::Span;
use crate::types::Type;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnSig {
    pub params: Vec<Type>,
    pub ret: Type,
}

/// Result of a successful analysis, consumed by the bytecode compiler.
#[derive(Debug, Default)]
pub struct Analysis {
    pub types: HashMap<ExprId, Type>,
    /// Signatures keyed by qualified name (`add`, `lib.math.square`).
    pub fns: HashMap<String, FnSig>,
    /// Resolved method of every method-call expression.
    pub methods: HashMap<ExprId, Method>,
    /// Qualified callee of every call to a user function, including module
    /// calls written `math.square(x)`.
    pub calls: HashMap<ExprId, String>,
}

#[derive(Clone)]
enum Global {
    Fn(Option<FnSig>),
    App,
}

struct Local {
    ty: Option<Type>,
    mutable: bool,
}

struct Checker {
    diags: Vec<Diagnostic>,
    types: HashMap<ExprId, Type>,
    methods: HashMap<ExprId, Method>,
    calls: HashMap<ExprId, String>,
    globals: HashMap<String, Global>,
    scopes: Vec<HashMap<String, Local>>,
    /// Module name of the unit being checked; empty for the main file.
    module: String,
    /// Imports visible in the unit being checked: alias to module name.
    imports: HashMap<String, String>,
    /// Return type of the function being checked; `None` inside an app.
    ret: Option<Type>,
    in_app: bool,
    loop_depth: usize,
}

/// Name of function `name` declared in `module`.
pub fn qualified(module: &str, name: &str) -> String {
    if module.is_empty() {
        name.to_string()
    } else {
        format!("{module}.{name}")
    }
}

/// Analyze a single-file program. `use` declarations are ignored here; the
/// project loader resolves them.
pub fn analyze(program: &Program) -> Result<Analysis, Vec<Diagnostic>> {
    analyze_units(&[Unit {
        name: String::new(),
        file: 0,
        program: program.clone(),
        imports: HashMap::new(),
    }])
}

/// Analyze every file of a project together.
pub fn analyze_units(units: &[Unit]) -> Result<Analysis, Vec<Diagnostic>> {
    let mut cx = Checker {
        diags: Vec::new(),
        types: HashMap::new(),
        methods: HashMap::new(),
        calls: HashMap::new(),
        globals: HashMap::new(),
        scopes: Vec::new(),
        module: String::new(),
        imports: HashMap::new(),
        ret: None,
        in_app: false,
        loop_depth: 0,
    };

    for unit in units {
        for item in &unit.program.items {
            match item {
                Item::Use(_) => {}
                Item::Fn(f) => {
                    let sig = cx.signature(f);
                    cx.declare_global(&unit.name, &f.name, Global::Fn(sig));
                }
                Item::App(a) => cx.declare_global(&unit.name, &a.name, Global::App),
            }
        }
    }

    for unit in units {
        cx.module = unit.name.clone();
        cx.imports = unit.imports.clone();
        for item in &unit.program.items {
            match item {
                Item::Fn(f) => cx.check_fn(f),
                Item::App(a) => cx.check_app(a),
                Item::Use(_) => {}
            }
        }
    }

    if !cx.diags.is_empty() {
        return Err(cx.diags);
    }
    let fns = cx
        .globals
        .into_iter()
        .filter_map(|(name, g)| match g {
            Global::Fn(Some(sig)) => Some((name, sig)),
            _ => None,
        })
        .collect();
    Ok(Analysis {
        types: cx.types,
        fns,
        methods: cx.methods,
        calls: cx.calls,
    })
}

/// Signature of a method on a receiver type: (method, params, result).
fn method_sig(recv: &Type, name: &str) -> Option<(Method, Vec<Type>, Type)> {
    use Method as M;
    let sig = match recv {
        Type::Array(elem) => {
            let t = (**elem).clone();
            let arr = recv.clone();
            match name {
                "len" => (M::Len, vec![], Type::Int),
                "push" => (M::Push, vec![t], Type::Unit),
                "pop" => (M::Pop, vec![], t),
                "insert" => (M::Insert, vec![Type::Int, t], Type::Unit),
                "remove_at" => (M::RemoveAt, vec![Type::Int], t),
                "clear" => (M::Clear, vec![], Type::Unit),
                "contains" => (M::Contains, vec![t], Type::Bool),
                "index_of" => (M::IndexOf, vec![t], Type::Int),
                "join" => (M::Join, vec![Type::Str], Type::Str),
                "sorted" if t.is_ordered() => (M::Sorted, vec![], arr),
                "reversed" => (M::Reversed, vec![], arr),
                "slice" => (M::Slice, vec![Type::Int, Type::Int], arr),
                _ => return None,
            }
        }
        Type::Str => match name {
            "len" => (M::Len, vec![], Type::Int),
            "upper" => (M::Upper, vec![], Type::Str),
            "lower" => (M::Lower, vec![], Type::Str),
            "trim" => (M::Trim, vec![], Type::Str),
            "contains" => (M::Contains, vec![Type::Str], Type::Bool),
            "index_of" => (M::IndexOf, vec![Type::Str], Type::Int),
            "starts_with" => (M::StartsWith, vec![Type::Str], Type::Bool),
            "ends_with" => (M::EndsWith, vec![Type::Str], Type::Bool),
            "split" => (M::Split, vec![Type::Str], Type::array(Type::Str)),
            "chars" => (M::Chars, vec![], Type::array(Type::Str)),
            "replace" => (M::Replace, vec![Type::Str, Type::Str], Type::Str),
            "slice" => (M::Slice, vec![Type::Int, Type::Int], Type::Str),
            _ => return None,
        },
        Type::Map(k, v) => {
            let (k, v) = ((**k).clone(), (**v).clone());
            match name {
                "len" => (M::Len, vec![], Type::Int),
                "has" => (M::Has, vec![k], Type::Bool),
                "get" => (M::Get, vec![k, v.clone()], v),
                "keys" => (M::Keys, vec![], Type::array(k)),
                "values" => (M::Values, vec![], Type::array(v)),
                "remove" => (M::Remove, vec![k], Type::Bool),
                "clear" => (M::Clear, vec![], Type::Unit),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(sig)
}

fn builtin_sig(b: Builtin) -> (Vec<Type>, Type) {
    match b {
        Builtin::ToFloat => (vec![Type::Int], Type::Float),
        Builtin::ToInt => (vec![Type::Float], Type::Int),
        Builtin::Sqrt | Builtin::Floor | Builtin::Round => (vec![Type::Float], Type::Float),
        Builtin::ParseInt => (vec![Type::Str], Type::Int),
        Builtin::ParseFloat => (vec![Type::Str], Type::Float),
        Builtin::Input => (vec![Type::Str], Type::Str),
        Builtin::ReadFile => (vec![Type::Str], Type::Str),
        Builtin::WriteFile | Builtin::AppendFile => (vec![Type::Str, Type::Str], Type::Unit),
        Builtin::FileExists => (vec![Type::Str], Type::Bool),
        Builtin::ListDir => (vec![Type::Str], Type::array(Type::Str)),
        Builtin::DeleteFile => (vec![Type::Str], Type::Unit),
        Builtin::Args => (vec![], Type::array(Type::Str)),
        Builtin::Env => (vec![Type::Str, Type::Str], Type::Str),
        Builtin::Exit => (vec![Type::Int], Type::Unit),
        Builtin::Eprint => (vec![Type::Str], Type::Unit),
        Builtin::TimeMs => (vec![], Type::Int),
        Builtin::SleepMs => (vec![Type::Int], Type::Unit),
    }
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

impl Checker {
    fn error(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    /// The function or app `name` as seen from the unit being checked.
    fn global(&self, name: &str) -> Option<Global> {
        self.globals.get(&qualified(&self.module, name)).cloned()
    }

    fn declare_global(&mut self, module: &str, name: &Ident, g: Global) {
        if Builtin::by_name(&name.name).is_some() {
            self.error(Diagnostic::error(
                format!(
                    "`{}` is a built-in function and cannot be redefined",
                    name.name
                ),
                name.span,
            ));
            return;
        }
        let key = qualified(module, &name.name);
        if self.globals.contains_key(&key) {
            self.error(
                Diagnostic::error(
                    format!("`{}` is defined more than once", name.name),
                    name.span,
                )
                .label("redefined here")
                .note("functions and apps share one namespace"),
            );
            return;
        }
        self.globals.insert(key, g);
    }

    fn resolve_type(&mut self, t: &TypeExpr) -> Option<Type> {
        match &t.kind {
            TypeExprKind::Named(name) => {
                let ty = Type::from_name(name);
                if ty.is_none() {
                    self.error(
                        Diagnostic::error(format!("unknown type `{name}`"), t.span).note(
                            "available types are Int, Float, Bool, Str, Unit, [T] and {K: V}",
                        ),
                    );
                }
                ty
            }
            TypeExprKind::Array(elem) => {
                let elem = self.resolve_type(elem)?;
                if elem == Type::Unit {
                    self.error(Diagnostic::error("arrays cannot hold `Unit`", t.span));
                    return None;
                }
                Some(Type::array(elem))
            }
            TypeExprKind::Map(k, v) => {
                let (kt, vt) = (self.resolve_type(k), self.resolve_type(v));
                let (kt, vt) = (kt?, vt?);
                if !kt.is_key() {
                    self.error(
                        Diagnostic::error(format!("`{kt}` cannot be a map key"), k.span)
                            .note("map keys must be Int, Bool or Str"),
                    );
                    return None;
                }
                if vt == Type::Unit {
                    self.error(Diagnostic::error("maps cannot hold `Unit`", v.span));
                    return None;
                }
                Some(Type::map(kt, vt))
            }
        }
    }

    fn signature(&mut self, f: &FnDecl) -> Option<FnSig> {
        let mut params = Vec::new();
        let mut ok = true;
        for p in &f.params {
            match self.resolve_type(&p.ty) {
                Some(Type::Unit) => {
                    self.error(Diagnostic::error(
                        "parameters cannot have type `Unit`",
                        p.ty.span,
                    ));
                    ok = false;
                }
                Some(t) => params.push(t),
                None => ok = false,
            }
        }
        let ret = match &f.ret {
            Some(t) => self.resolve_type(t),
            None => Some(Type::Unit),
        };
        match (ok, ret) {
            (true, Some(ret)) => Some(FnSig { params, ret }),
            _ => None,
        }
    }

    fn check_fn(&mut self, f: &FnDecl) {
        let sig = match self.global(&f.name.name) {
            Some(Global::Fn(sig)) => sig,
            _ => None,
        };
        let declared_ret = sig.as_ref().map(|s| s.ret.clone());
        self.ret = declared_ret.clone();
        self.in_app = false;
        self.loop_depth = 0;
        self.scopes = vec![HashMap::new()];
        for (i, p) in f.params.iter().enumerate() {
            let ty = sig.as_ref().and_then(|s| s.params.get(i).cloned());
            self.declare_local(&p.name, ty, false);
        }
        self.check_stmts(&f.body.stmts);
        self.scopes.clear();

        if let Some(ret) = declared_ret {
            if ret != Type::Unit && !block_returns(&f.body) {
                self.error(
                    Diagnostic::error(
                        format!(
                            "function `{}` may reach its end without returning a value of type `{ret}`",
                            f.name.name
                        ),
                        f.name.span,
                    )
                    .note("every path must end with `return <value>`; loops are not assumed to return"),
                );
            }
        }
    }

    fn check_app(&mut self, a: &AppDecl) {
        self.ret = None;
        self.in_app = true;
        self.loop_depth = 0;
        self.scopes = vec![HashMap::new()];
        self.check_stmts(&a.body.stmts);
        self.scopes.clear();
    }

    fn declare_local(&mut self, name: &Ident, ty: Option<Type>, mutable: bool) {
        let scope = self.scopes.last_mut().expect("no scope");
        if scope.contains_key(&name.name) {
            self.diags.push(
                Diagnostic::error(
                    format!("`{}` is already declared in this scope", name.name),
                    name.span,
                )
                .note("use a new name, or declare it with `var` and assign to it"),
            );
            return;
        }
        scope.insert(name.name.clone(), Local { ty, mutable });
    }

    fn lookup(&self, name: &str) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn check_block(&mut self, b: &Block) {
        self.scopes.push(HashMap::new());
        self.check_stmts(&b.stmts);
        self.scopes.pop();
    }

    fn check_stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.check_stmt(s);
        }
    }

    fn check_stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                value,
            } => {
                let declared = ty.as_ref().and_then(|t| self.resolve_type(t));
                let ty = match &declared {
                    Some(d) => {
                        self.expect_expr(value, d);
                        Some(d.clone())
                    }
                    None if ty.is_some() => {
                        self.expr(value, None);
                        None
                    }
                    None => self.expr(value, None),
                };
                let ty = if ty == Some(Type::Unit) {
                    self.error(Diagnostic::error(
                        "cannot store a `Unit` value in a variable",
                        value.span,
                    ));
                    None
                } else {
                    ty
                };
                self.declare_local(name, ty, *mutable);
            }
            StmtKind::Assign { name, path, value } => self.check_assign(name, path, value),
            StmtKind::Say(e) => {
                if self.expr(e, None) == Some(Type::Unit) {
                    self.error(Diagnostic::error(
                        "cannot `say` a value of type `Unit`",
                        e.span,
                    ));
                }
            }
            StmtKind::Return(value) => self.check_return(value.as_ref(), s.span),
            StmtKind::If { cond, then, else_ } => {
                self.expect_cond(cond, "`if` condition");
                self.check_block(then);
                match else_ {
                    Some(ElseBranch::Block(b)) => self.check_block(b),
                    Some(ElseBranch::If(stmt)) => self.check_stmt(stmt),
                    None => {}
                }
            }
            StmtKind::While { cond, body } => {
                self.expect_cond(cond, "`while` condition");
                self.loop_body(body, None);
            }
            StmtKind::For { var, iter, body } => {
                let elem = match iter {
                    ForIter::Range(start, end) => {
                        self.expect_expr(start, &Type::Int);
                        self.expect_expr(end, &Type::Int);
                        Some(Type::Int)
                    }
                    ForIter::Each(e) => match self.expr(e, None) {
                        Some(Type::Array(elem)) => Some(*elem),
                        Some(Type::Map(k, _)) => Some(*k),
                        Some(other) => {
                            self.error(
                                Diagnostic::error(
                                    format!("cannot loop over a value of type `{other}`"),
                                    e.span,
                                )
                                .note("`for` works on arrays, maps (keys) and ranges like `0..10`"),
                            );
                            None
                        }
                        None => None,
                    },
                };
                self.loop_body(body, Some((var, elem)));
            }
            StmtKind::Try { body, err, handler } => {
                self.check_block(body);
                self.scopes.push(HashMap::new());
                self.declare_local(err, Some(Type::Str), false);
                self.check_stmts(&handler.stmts);
                self.scopes.pop();
            }
            StmtKind::Fail(message) => self.expect_expr(message, &Type::Str),
            StmtKind::Break | StmtKind::Continue => {
                if self.loop_depth == 0 {
                    let word = if matches!(s.kind, StmtKind::Break) {
                        "break"
                    } else {
                        "continue"
                    };
                    self.error(Diagnostic::error(
                        format!("`{word}` outside of a loop"),
                        s.span,
                    ));
                }
            }
            StmtKind::Block(b) => self.check_block(b),
            StmtKind::Expr(e) => {
                self.expr(e, None);
            }
        }
    }

    fn loop_body(&mut self, body: &Block, var: Option<(&Ident, Option<Type>)>) {
        self.loop_depth += 1;
        self.scopes.push(HashMap::new());
        if let Some((name, ty)) = var {
            self.declare_local(name, ty, false);
        }
        self.check_stmts(&body.stmts);
        self.scopes.pop();
        self.loop_depth -= 1;
    }

    fn check_assign(&mut self, name: &Ident, path: &[Expr], value: &Expr) {
        let Some(local) = self.lookup(&name.name) else {
            self.error(Diagnostic::error(
                format!("cannot find variable `{}` in this scope", name.name),
                name.span,
            ));
            for p in path {
                self.expr(p, None);
            }
            self.expr(value, None);
            return;
        };
        let (mut target, mutable) = (local.ty.clone(), local.mutable);
        if !mutable {
            let what = if path.is_empty() {
                format!("cannot assign twice to immutable variable `{}`", name.name)
            } else {
                format!(
                    "cannot change an element of immutable variable `{}`",
                    name.name
                )
            };
            self.error(Diagnostic::error(what, name.span).note(format!(
                "declare it with `var {}` to make it mutable",
                name.name
            )));
        }
        for index in path {
            target = match target {
                Some(Type::Array(elem)) => {
                    self.expect_expr(index, &Type::Int);
                    Some(*elem)
                }
                Some(Type::Map(k, v)) => {
                    self.expect_expr(index, &k);
                    Some(*v)
                }
                Some(other) => {
                    self.expr(index, None);
                    self.error(Diagnostic::error(
                        format!("cannot assign into a value of type `{other}` by index"),
                        index.span,
                    ));
                    None
                }
                None => {
                    self.expr(index, None);
                    None
                }
            };
        }
        match target {
            Some(t) => self.expect_expr(value, &t),
            None => {
                self.expr(value, None);
            }
        }
    }

    fn check_return(&mut self, value: Option<&Expr>, span: Span) {
        if self.in_app {
            if let Some(v) = value {
                self.expr(v, None);
                self.error(
                    Diagnostic::error("an `app` block cannot return a value", v.span)
                        .note("use a bare `return` to leave the app early"),
                );
            }
            return;
        }
        let ret = self.ret.clone();
        match (value, ret) {
            (Some(v), Some(Type::Unit)) => {
                let found = self.expr(v, None);
                let mut d = Diagnostic::error(
                    "this function has no return type, so it cannot return a value",
                    v.span,
                );
                if let Some(found) = found {
                    d = d.note(format!("add `-> {found}` to the function signature"));
                }
                self.error(d);
            }
            (Some(v), Some(want)) => self.expect_expr(v, &want),
            (Some(v), None) => {
                self.expr(v, None);
            }
            (None, Some(want)) if want != Type::Unit => {
                self.error(Diagnostic::error(
                    format!("missing return value of type `{want}`"),
                    span,
                ));
            }
            (None, _) => {}
        }
    }

    fn mismatch(&mut self, expected: &Type, found: &Type, span: Span) {
        self.error(
            Diagnostic::error(
                format!("mismatched types: expected `{expected}`, found `{found}`"),
                span,
            )
            .label(format!("this is `{found}`")),
        );
    }

    /// Check `e` against an expected type and report a mismatch.
    fn expect_expr(&mut self, e: &Expr, want: &Type) {
        if let Some(found) = self.expr(e, Some(want)) {
            if &found != want {
                self.mismatch(want, &found, e.span);
            }
        }
    }

    fn expect_cond(&mut self, e: &Expr, what: &str) {
        if let Some(found) = self.expr(e, Some(&Type::Bool)) {
            if found != Type::Bool {
                self.error(
                    Diagnostic::error(format!("{what} must be `Bool`, found `{found}`"), e.span)
                        .label(format!("this is `{found}`")),
                );
            }
        }
    }

    /// Type an expression. `expected` only guides empty collection literals;
    /// callers still compare the result.
    fn expr(&mut self, e: &Expr, expected: Option<&Type>) -> Option<Type> {
        let ty = self.expr_inner(e, expected);
        if let Some(t) = &ty {
            self.types.insert(e.id, t.clone());
        }
        ty
    }

    fn expr_inner(&mut self, e: &Expr, expected: Option<&Type>) -> Option<Type> {
        match &e.kind {
            ExprKind::Int(_) => Some(Type::Int),
            ExprKind::Float(_) => Some(Type::Float),
            ExprKind::Bool(_) => Some(Type::Bool),
            ExprKind::Str(_) => Some(Type::Str),
            ExprKind::Interp(parts) => {
                for part in parts {
                    if let InterpPart::Expr(inner) = part {
                        if self.expr(inner, None) == Some(Type::Unit) {
                            self.error(Diagnostic::error(
                                "cannot interpolate a value of type `Unit`",
                                inner.span,
                            ));
                        }
                    }
                }
                Some(Type::Str)
            }
            ExprKind::Array(items) => self.array_literal(e, items, expected),
            ExprKind::Map(entries) => self.map_literal(e, entries, expected),
            ExprKind::Var(name) => {
                if let Some(local) = self.lookup(name) {
                    return local.ty.clone();
                }
                let d = match self.global(name) {
                    Some(Global::Fn(_)) => {
                        Diagnostic::error(format!("`{name}` is a function, not a value"), e.span)
                            .note(format!("call it with `{name}(...)`"))
                    }
                    Some(Global::App) => {
                        Diagnostic::error(format!("`{name}` is an app, not a value"), e.span)
                    }
                    None if self.imports.contains_key(name) => {
                        Diagnostic::error(format!("`{name}` is a module, not a value"), e.span)
                            .note(format!("call its functions like `{name}.function(...)`"))
                    }
                    None => Diagnostic::error(
                        format!("cannot find variable `{name}` in this scope"),
                        e.span,
                    ),
                };
                self.error(d);
                None
            }
            ExprKind::Index { base, index } => {
                let bt = self.expr(base, None);
                match bt? {
                    Type::Array(elem) => {
                        self.expect_expr(index, &Type::Int);
                        Some(*elem)
                    }
                    Type::Map(k, v) => {
                        self.expect_expr(index, &k);
                        Some(*v)
                    }
                    Type::Str => {
                        self.expr(index, None);
                        self.error(
                            Diagnostic::error("strings cannot be indexed with `[]`", e.span).note(
                                "use `.chars()` for a list of characters, or `.slice(start, end)`",
                            ),
                        );
                        None
                    }
                    other => {
                        self.expr(index, None);
                        self.error(Diagnostic::error(
                            format!("cannot index into a value of type `{other}`"),
                            e.span,
                        ));
                        None
                    }
                }
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => self.method_call(e, receiver, method, args),
            ExprKind::Unary { op, expr } => {
                let t = self.expr(expr, None)?;
                let ok = match op {
                    UnOp::Neg => matches!(t, Type::Int | Type::Float),
                    UnOp::Not => t == Type::Bool,
                };
                if !ok {
                    let sym = if *op == UnOp::Neg { "-" } else { "!" };
                    self.error(Diagnostic::error(
                        format!("cannot apply unary `{sym}` to `{t}`"),
                        e.span,
                    ));
                    return None;
                }
                Some(t)
            }
            ExprKind::Binary {
                op,
                op_span,
                lhs,
                rhs,
            } => {
                let l = self.expr(lhs, None);
                let r = self.expr(rhs, l.as_ref());
                let (l, r) = (l?, r?);
                let numeric = |t: &Type| matches!(t, Type::Int | Type::Float);
                let result = match op {
                    BinOp::Add
                        if l == r && (numeric(&l) || matches!(l, Type::Str | Type::Array(_))) =>
                    {
                        Some(l.clone())
                    }
                    BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem if l == r && numeric(&l) => {
                        Some(l.clone())
                    }
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge if l == r && l.is_ordered() => {
                        Some(Type::Bool)
                    }
                    BinOp::Eq | BinOp::Ne if l == r && !l.contains_unit() => Some(Type::Bool),
                    BinOp::And | BinOp::Or if l == Type::Bool && r == Type::Bool => {
                        Some(Type::Bool)
                    }
                    _ => None,
                };
                if result.is_none() {
                    let mut d = Diagnostic::error(
                        format!("cannot apply `{}` to `{l}` and `{r}`", op.symbol()),
                        *op_span,
                    );
                    if numeric(&l) && numeric(&r) && l != r {
                        d = d.note(
                            "Int and Float do not mix; convert with `to_float(n)` or `to_int(x)`",
                        );
                    } else if *op == BinOp::Add && (l == Type::Str || r == Type::Str) {
                        d = d.note("to build a string from other values, use interpolation: \"total: {n}\"");
                    }
                    self.error(d);
                }
                result
            }
            ExprKind::Call { callee, args } => self.call(e, callee, args),
        }
    }

    fn array_literal(&mut self, e: &Expr, items: &[Expr], expected: Option<&Type>) -> Option<Type> {
        let hint = match expected {
            Some(Type::Array(elem)) => Some((**elem).clone()),
            _ => None,
        };
        let Some(first) = items.first() else {
            return match hint {
                Some(elem) => Some(Type::array(elem)),
                None => {
                    self.error(
                        Diagnostic::error("cannot infer the type of an empty array", e.span)
                            .note("add a type annotation, e.g. `let items: [Int] = []`"),
                    );
                    None
                }
            };
        };
        let elem = match &hint {
            Some(h) => {
                self.expect_expr(first, h);
                Some(h.clone())
            }
            None => self.expr(first, None),
        };
        for item in &items[1..] {
            match &elem {
                Some(t) => self.expect_expr(item, t),
                None => {
                    self.expr(item, None);
                }
            }
        }
        let elem = elem?;
        if elem == Type::Unit {
            self.error(Diagnostic::error("arrays cannot hold `Unit`", first.span));
            return None;
        }
        Some(Type::array(elem))
    }

    fn map_literal(
        &mut self,
        e: &Expr,
        entries: &[(Expr, Expr)],
        expected: Option<&Type>,
    ) -> Option<Type> {
        let hint = match expected {
            Some(Type::Map(k, v)) => Some(((**k).clone(), (**v).clone())),
            _ => None,
        };
        let Some((k0, v0)) = entries.first() else {
            return match hint {
                Some((k, v)) => Some(Type::map(k, v)),
                None => {
                    self.error(
                        Diagnostic::error("cannot infer the type of an empty map", e.span)
                            .note("add a type annotation, e.g. `let ages: {Str: Int} = {}`"),
                    );
                    None
                }
            };
        };
        let (kt, vt) = match &hint {
            Some((k, v)) => {
                self.expect_expr(k0, k);
                self.expect_expr(v0, v);
                (Some(k.clone()), Some(v.clone()))
            }
            None => (self.expr(k0, None), self.expr(v0, None)),
        };
        for (k, v) in &entries[1..] {
            match &kt {
                Some(t) => self.expect_expr(k, t),
                None => {
                    self.expr(k, None);
                }
            }
            match &vt {
                Some(t) => self.expect_expr(v, t),
                None => {
                    self.expr(v, None);
                }
            }
        }
        let (kt, vt) = (kt?, vt?);
        if !kt.is_key() {
            self.error(
                Diagnostic::error(format!("`{kt}` cannot be a map key"), k0.span)
                    .note("map keys must be Int, Bool or Str"),
            );
            return None;
        }
        if vt == Type::Unit {
            self.error(Diagnostic::error("maps cannot hold `Unit`", v0.span));
            return None;
        }
        Some(Type::map(kt, vt))
    }

    fn check_args(&mut self, what: &str, call: &Expr, args: &[Expr], params: &[Type]) {
        if params.len() != args.len() {
            for a in args {
                self.expr(a, None);
            }
            self.error(Diagnostic::error(
                format!(
                    "{what} takes {} but {} {} given",
                    plural(params.len(), "argument"),
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" }
                ),
                call.span,
            ));
            return;
        }
        for (arg, want) in args.iter().zip(params) {
            self.expect_expr(arg, want);
        }
    }

    fn call(&mut self, e: &Expr, callee: &Ident, args: &[Expr]) -> Option<Type> {
        if let Some(b) = Builtin::by_name(&callee.name) {
            let (params, ret) = builtin_sig(b);
            self.check_args(&format!("function `{}`", callee.name), e, args, &params);
            return Some(ret);
        }
        let sig = match self.global(&callee.name) {
            Some(Global::Fn(Some(sig))) => sig,
            Some(Global::Fn(None)) => {
                for a in args {
                    self.expr(a, None);
                }
                return None;
            }
            Some(Global::App) => {
                self.error(Diagnostic::error(
                    format!("`{}` is an app and cannot be called", callee.name),
                    callee.span,
                ));
                return None;
            }
            None => {
                for a in args {
                    self.expr(a, None);
                }
                let mut d = Diagnostic::error(
                    format!("cannot find function `{}`", callee.name),
                    callee.span,
                );
                if self.lookup(&callee.name).is_some() {
                    d = d.note(format!("`{}` is a variable, not a function", callee.name));
                }
                self.error(d);
                return None;
            }
        };
        self.check_args(&format!("function `{}`", callee.name), e, args, &sig.params);
        self.calls
            .insert(e.id, qualified(&self.module, &callee.name));
        Some(sig.ret)
    }

    /// `alias.function(args)` where `alias` names an imported module.
    fn module_call(
        &mut self,
        e: &Expr,
        module: &str,
        alias: &str,
        function: &Ident,
        args: &[Expr],
    ) -> Option<Type> {
        let target = qualified(module, &function.name);
        let Some(Global::Fn(sig)) = self.globals.get(&target).cloned() else {
            for a in args {
                self.expr(a, None);
            }
            self.error(Diagnostic::error(
                format!("module `{alias}` has no function `{}`", function.name),
                function.span,
            ));
            return None;
        };
        let Some(sig) = sig else {
            for a in args {
                self.expr(a, None);
            }
            return None;
        };
        self.check_args(
            &format!("function `{alias}.{}`", function.name),
            e,
            args,
            &sig.params,
        );
        self.calls.insert(e.id, target);
        Some(sig.ret)
    }

    fn method_call(
        &mut self,
        e: &Expr,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
    ) -> Option<Type> {
        if let ExprKind::Var(alias) = &receiver.kind {
            if self.lookup(alias).is_none() {
                if let Some(module) = self.imports.get(alias).cloned() {
                    return self.module_call(e, &module, alias, method, args);
                }
            }
        }
        let Some(rt) = self.expr(receiver, None) else {
            for a in args {
                self.expr(a, None);
            }
            return None;
        };
        let Some((m, params, ret)) = method_sig(&rt, &method.name) else {
            for a in args {
                self.expr(a, None);
            }
            let mut d = Diagnostic::error(
                format!("type `{rt}` has no method `{}`", method.name),
                method.span,
            );
            if method.name == "sorted" {
                d = d.note("only arrays of Int, Float or Str can be sorted");
            }
            self.error(d);
            return None;
        };
        if m.mutating() {
            match &receiver.kind {
                ExprKind::Var(name) => {
                    if self.lookup(name).is_some_and(|l| !l.mutable) {
                        self.error(
                            Diagnostic::error(
                                format!(
                                    "cannot call `{}` on immutable variable `{name}`",
                                    method.name
                                ),
                                receiver.span,
                            )
                            .note(format!("declare it with `var {name}` to make it mutable")),
                        );
                    }
                }
                _ => {
                    self.error(
                        Diagnostic::error(
                            format!(
                                "`{}` changes its receiver, so it must be called on a variable",
                                method.name
                            ),
                            receiver.span,
                        )
                        .note("copy the value into a `var` first, change it, then store it back"),
                    );
                }
            }
        }
        self.check_args(&format!("method `{}`", method.name), e, args, &params);
        self.methods.insert(e.id, m);
        Some(ret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn errors(src: &str) -> Vec<String> {
        let program = parse(src).expect("parse");
        match analyze(&program) {
            Ok(_) => Vec::new(),
            Err(diags) => diags.into_iter().map(|d| d.message).collect(),
        }
    }

    fn assert_ok(src: &str) {
        assert_eq!(errors(src), Vec::<String>::new(), "{src}");
    }

    fn assert_error(src: &str, want: &str) {
        let errs = errors(src);
        assert!(
            errs.iter().any(|e| e.contains(want)),
            "{src:?}: errors {errs:?} do not contain {want:?}"
        );
    }

    #[test]
    fn accepts_valid_program() {
        assert_ok(
            r#"
fn add(a: Int, b: Int) -> Int { return a + b }
fn greet(name: Str) { say "hi {name}" }
fn sign(n: Int) -> Int {
    if n < 0 { return -1 } else if n == 0 { return 0 } else { return 1 }
}
app Main {
    var total = 0
    let label: Str = "sum"
    while total < 10 { total = total + add(1, 2) }
    greet(label + "!")
    say sign(total) == 1 && !false
}
"#,
        );
    }

    #[test]
    fn accepts_collections_and_floats() {
        assert_ok(
            r#"
fn total(xs: [Float]) -> Float {
    var sum = 0.0
    for x in xs { sum = sum + x }
    return sum
}
app Main {
    var xs: [Int] = []
    xs.push(3)
    xs[0] = xs[0] * 2
    var grid = [[1, 2], [3, 4]]
    grid[1][0] = 9
    var ages: {Str: Int} = {}
    ages["ana"] = 30
    let names = ages.keys().sorted()
    for i in 0..xs.len() { say xs[i] }
    for k in ages { say "{k}={ages[k]}" }
    say total([1.5, 2.5]) / 2.0
    say "a,b".split(",").join("-").upper()
    say to_float(1) + sqrt(4.0)
    say ages.get("bob", 0) + names.len()
    say [1, 2] + [3] == [1, 2, 3]
    say "b" > "a"
}
"#,
        );
    }

    #[test]
    fn reports_collection_errors() {
        assert_error("app A { let xs = [] }", "empty array");
        assert_error("app A { let m = {} }", "empty map");
        assert_error("app A { say [1, \"a\"] }", "expected `Int`, found `Str`");
        assert_error(
            "app A { let xs = [1]\n xs.push(2) }",
            "immutable variable `xs`",
        );
        assert_error("app A { let xs = [1]\n xs[0] = 2 }", "element of immutable");
        assert_error(
            "app A { var xs = [1]\n xs[\"a\"] = 2 }",
            "expected `Int`, found `Str`",
        );
        assert_error("app A { say [1].push(2) }", "must be called on a variable");
        assert_error("app A { say [true].sorted() }", "has no method `sorted`");
        assert_error("app A { say 5.len() }", "has no method `len`");
        assert_error("app A { say \"abc\"[0] }", "strings cannot be indexed");
        assert_error("app A { let m = {[1]: 2} }", "cannot be a map key");
        assert_error("fn f(m: {Float: Int}) {}", "cannot be a map key");
        assert_error("app A { for x in 5 { } }", "cannot loop over");
        assert_error(
            "app A { for i in 0..\"a\" { } }",
            "expected `Int`, found `Str`",
        );
        assert_error("app A { for x in [1] { x = 2 } }", "immutable variable `x`");
        assert_error(
            "app A { say [1].slice(0) }",
            "takes 2 arguments but 1 was given",
        );
    }

    #[test]
    fn try_catch_and_fail() {
        assert_ok(
            "fn f(x: Int) -> Int {\n if x > 0 { return x }\n fail \"negative\"\n}\napp A {\n try { say f(1) } catch e { say e.upper() }\n}",
        );
        assert_error("app A { fail 1 }", "expected `Str`, found `Int`");
        assert_error(
            "app A { try { } catch e { e = \"x\" } }",
            "immutable variable `e`",
        );
        assert_error(
            "app A { try { } catch e { }\n say e }",
            "cannot find variable `e`",
        );
        // `try` only counts as returning when both blocks do
        assert_error(
            "fn f() -> Int { try { return 1 } catch e { say e } }",
            "may reach its end without returning",
        );
        assert_ok("fn f() -> Int { try { return 1 } catch e { return 2 } }");
    }

    #[test]
    fn module_calls() {
        use crate::parser::parse_file;
        let (main, next) = parse_file(
            "use lib.m\napp A { say m.twice(2)\n say m.nope()\n say m }",
            0,
            0,
        )
        .unwrap();
        let (lib, _) = parse_file("fn twice(n: Int) -> Int { return helper(n) }\nfn helper(n: Int) -> Int { return n * 2 }", 1, next).unwrap();
        let units = vec![
            Unit {
                name: String::new(),
                file: 0,
                program: main,
                imports: [("m".to_string(), "lib.m".to_string())].into(),
            },
            Unit {
                name: "lib.m".into(),
                file: 1,
                program: lib,
                imports: HashMap::new(),
            },
        ];
        let errs: Vec<String> = analyze_units(&units)
            .unwrap_err()
            .into_iter()
            .map(|d| d.message)
            .collect();
        assert_eq!(
            errs,
            [
                "module `m` has no function `nope`",
                "`m` is a module, not a value"
            ]
        );
    }

    #[test]
    fn reports_float_errors() {
        assert_error(
            "app A { say 1 + 1.5 }",
            "cannot apply `+` to `Int` and `Float`",
        );
        assert_error("app A { say to_int(3) }", "expected `Float`, found `Int`");
        assert_error("fn sqrt(x: Int) {}", "built-in function");
        assert_error("app A { say -\"a\" }", "unary `-`");
    }

    #[test]
    fn reports_name_errors() {
        assert_error("app A { say x }", "cannot find variable `x`");
        assert_error("app A { say f(1) }", "cannot find function `f`");
        assert_error("fn f() {}\napp A { say f }", "is a function, not a value");
        assert_error("app A {}\napp B { A() }", "is an app and cannot be called");
        assert_error("app A {}\napp A {}", "defined more than once");
        assert_error("app A { let x = 1\n let x = 2 }", "already declared");
        assert_error("app A { let x = 1\n x = 2 }", "immutable variable `x`");
    }

    #[test]
    fn shadowing_in_inner_scope_is_allowed() {
        assert_ok("app A { let x = 1\n { let x = \"s\"\n say x }\n say x }");
        assert_error(
            "app A { { let y = 1 }\n say y }",
            "cannot find variable `y`",
        );
    }

    #[test]
    fn reports_type_errors() {
        assert_error(
            "app A { say 1 + \"a\" }",
            "cannot apply `+` to `Int` and `Str`",
        );
        assert_error("app A { say true < false }", "cannot apply `<`");
        assert_error("app A { say 1 == true }", "cannot apply `==`");
        assert_error("app A { if 1 { } }", "`if` condition must be `Bool`");
        assert_error(
            "app A { while \"x\" { } }",
            "`while` condition must be `Bool`",
        );
        assert_error(
            "app A { let x: Int = true }",
            "expected `Int`, found `Bool`",
        );
        assert_error(
            "app A { var x = 1\n x = \"s\" }",
            "expected `Int`, found `Str`",
        );
        assert_error("app A { say -true }", "unary `-`");
        assert_error("app A { say !1 }", "unary `!`");
        assert_error(
            "fn f(a: Int) {}\napp A { f(\"s\") }",
            "expected `Int`, found `Str`",
        );
        assert_error(
            "fn f(a: Int) {}\napp A { f() }",
            "takes 1 argument but 0 were given",
        );
        assert_error(
            "fn f() {}\napp A { say f() }",
            "cannot `say` a value of type `Unit`",
        );
        assert_error("fn f() {}\napp A { let x = f() }", "cannot store a `Unit`");
        assert_error("fn f() {}\napp A { say \"{f()}\" }", "cannot interpolate");
        assert_error("fn f(a: Real) {}", "unknown type `Real`");
        assert_error("fn f(a: Unit) {}", "cannot have type `Unit`");
    }

    #[test]
    fn reports_control_flow_errors() {
        assert_error("app A { break }", "`break` outside of a loop");
        assert_error("app A { continue }", "`continue` outside of a loop");
        assert_error("app A { return 1 }", "cannot return a value");
        assert_error("fn f() -> Int { }", "may reach its end without returning");
        assert_error(
            "fn f(x: Bool) -> Int { if x { return 1 } }",
            "may reach its end without returning",
        );
        assert_error(
            "fn f() -> Int { while true { return 1 } }",
            "may reach its end without returning",
        );
        assert_error("fn f() -> Int { return }", "missing return value");
        assert_error(
            "fn f() -> Int { return true }",
            "expected `Int`, found `Bool`",
        );
        assert_error("fn f() { return 1 }", "no return type");
    }

    #[test]
    fn collects_multiple_errors() {
        let errs = errors("app A { say a\n say b\n say 1 + true }");
        assert_eq!(errs.len(), 3, "{errs:?}");
    }

    #[test]
    fn unknown_operand_does_not_cascade() {
        let errs = errors("app A { say x + 1 + 2 }");
        assert_eq!(errs.len(), 1, "{errs:?}");
    }
}
