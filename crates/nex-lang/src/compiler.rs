//! Bytecode compiler: checked AST to a bytecode module.
//!
//! Assumes the program passed semantic analysis; the resulting module is
//! still run through the verifier before execution.

use crate::ast::*;
use crate::bytecode::{Builtin, Const, FnKind, Function, Method, Module, Op, Pos};
use crate::sema::Analysis;
use crate::span::SourceFile;
use crate::types::Type;
use std::collections::HashMap;

pub fn compile(program: &Program, analysis: &Analysis, src: &SourceFile) -> Module {
    let mut fn_index = HashMap::new();
    let mut next = 0u32;
    for item in &program.items {
        match item {
            Item::Fn(f) => {
                fn_index.insert(f.name.name.clone(), next);
                next += 1;
            }
            Item::App(_) => next += 1,
            Item::Use(_) => {}
        }
    }

    let mut consts = Consts::default();
    let mut functions = Vec::new();
    let mut apps = Vec::new();
    for item in &program.items {
        let mut b = FnBuilder {
            analysis,
            src,
            fn_index: &fn_index,
            consts: &mut consts,
            code: Vec::new(),
            positions: Vec::new(),
            scopes: vec![HashMap::new()],
            num_locals: 0,
            loops: Vec::new(),
        };
        let function = match item {
            Item::Fn(f) => {
                for p in &f.params {
                    b.declare(&p.name.name);
                }
                b.block_stmts(&f.body.stmts);
                let returns_unit = analysis
                    .fns
                    .get(&f.name.name)
                    .is_none_or(|sig| sig.ret == Type::Unit);
                if returns_unit || !block_diverges(&f.body) {
                    b.emit(Op::PushUnit, f.body.span.end.saturating_sub(1));
                    b.emit(Op::Return, f.body.span.end.saturating_sub(1));
                }
                b.finish(&f.name.name, FnKind::Fn, f.params.len() as u32)
            }
            Item::App(a) => {
                b.block_stmts(&a.body.stmts);
                b.emit(Op::PushUnit, a.body.span.end.saturating_sub(1));
                b.emit(Op::Return, a.body.span.end.saturating_sub(1));
                apps.push(functions.len() as u32);
                b.finish(&a.name.name, FnKind::App, 0)
            }
            Item::Use(_) => continue,
        };
        functions.push(function);
    }

    Module {
        source_name: src.name.clone(),
        consts: consts.list,
        functions,
        apps,
    }
}

#[derive(Default)]
struct Consts {
    list: Vec<Const>,
    ints: HashMap<i64, u32>,
    floats: HashMap<u64, u32>,
    strs: HashMap<String, u32>,
}

impl Consts {
    fn add(&mut self, c: Const) -> u32 {
        self.list.push(c);
        (self.list.len() - 1) as u32
    }

    fn int(&mut self, v: i64) -> u32 {
        if let Some(&i) = self.ints.get(&v) {
            return i;
        }
        let i = self.add(Const::Int(v));
        self.ints.insert(v, i);
        i
    }

    fn float(&mut self, v: f64) -> u32 {
        if let Some(&i) = self.floats.get(&v.to_bits()) {
            return i;
        }
        let i = self.add(Const::Float(v));
        self.floats.insert(v.to_bits(), i);
        i
    }

    fn str(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.strs.get(s) {
            return i;
        }
        let i = self.add(Const::Str(s.to_string()));
        self.strs.insert(s.to_string(), i);
        i
    }
}

struct LoopCx {
    /// Where `continue` jumps; `None` until the loop's step code is emitted.
    continue_to: Option<u32>,
    continues: Vec<usize>,
    breaks: Vec<usize>,
}

struct FnBuilder<'a> {
    analysis: &'a Analysis,
    src: &'a SourceFile,
    fn_index: &'a HashMap<String, u32>,
    consts: &'a mut Consts,
    code: Vec<Op>,
    positions: Vec<Pos>,
    scopes: Vec<HashMap<String, u32>>,
    num_locals: u32,
    loops: Vec<LoopCx>,
}

impl FnBuilder<'_> {
    fn finish(self, name: &str, kind: FnKind, arity: u32) -> Function {
        Function {
            name: name.to_string(),
            kind,
            arity,
            num_locals: self.num_locals,
            code: self.code,
            positions: self.positions,
        }
    }

    fn emit(&mut self, op: Op, offset: usize) -> usize {
        let (line, col) = self.src.line_col(offset);
        self.code.push(op);
        self.positions.push(Pos {
            line: line as u32,
            col: col as u32,
        });
        self.code.len() - 1
    }

    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    /// Point the jump at `at` to the current position.
    fn patch(&mut self, at: usize) {
        let target = self.here();
        self.code[at] = match self.code[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            other => unreachable!("patching non-jump {other:?}"),
        };
    }

    fn declare(&mut self, name: &str) -> u32 {
        let slot = self.num_locals;
        self.num_locals += 1;
        self.scopes
            .last_mut()
            .expect("no scope")
            .insert(name.to_string(), slot);
        slot
    }

    /// A local slot with no source name, for loop bookkeeping.
    fn hidden(&mut self) -> u32 {
        let slot = self.num_locals;
        self.num_locals += 1;
        slot
    }

    fn slot(&self, name: &str) -> u32 {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(name).copied())
            .unwrap_or_else(|| panic!("unresolved variable `{name}` reached the compiler"))
    }

    fn block(&mut self, b: &Block) {
        self.scopes.push(HashMap::new());
        self.block_stmts(&b.stmts);
        self.scopes.pop();
    }

    /// Compile statements, dropping any that follow a diverging statement
    /// (they are unreachable).
    fn block_stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.stmt(s);
            if stmt_diverges(s) {
                break;
            }
        }
    }

    fn push_int(&mut self, v: i64, at: usize) {
        let i = self.consts.int(v);
        self.emit(Op::PushConst(i), at);
    }

    fn stmt(&mut self, s: &Stmt) {
        let at = s.span.start;
        match &s.kind {
            StmtKind::Let { name, value, .. } => {
                self.expr(value);
                let slot = self.declare(&name.name);
                self.emit(Op::Store(slot), name.span.start);
            }
            StmtKind::Assign { name, path, value } => {
                let slot = self.slot(&name.name);
                for index in path {
                    self.expr(index);
                }
                self.expr(value);
                if path.is_empty() {
                    self.emit(Op::Store(slot), name.span.start);
                } else {
                    self.emit(Op::StoreIndex(slot, path.len() as u32), name.span.start);
                }
            }
            StmtKind::Say(e) => {
                self.expr(e);
                self.emit(Op::Say, at);
            }
            StmtKind::Return(value) => {
                match value {
                    Some(v) => self.expr(v),
                    None => {
                        self.emit(Op::PushUnit, at);
                    }
                }
                self.emit(Op::Return, at);
            }
            StmtKind::If { cond, then, else_ } => {
                self.expr(cond);
                let jf = self.emit(Op::JumpIfFalse(0), cond.span.start);
                self.block(then);
                match else_ {
                    None => self.patch(jf),
                    Some(e) => {
                        let jend = (!block_diverges(then)).then(|| self.emit(Op::Jump(0), at));
                        self.patch(jf);
                        match e {
                            ElseBranch::Block(b) => self.block(b),
                            ElseBranch::If(stmt) => self.stmt(stmt),
                        }
                        if let Some(j) = jend {
                            self.patch(j);
                        }
                    }
                }
            }
            StmtKind::While { cond, body } => {
                let start = self.here();
                self.expr(cond);
                let jf = self.emit(Op::JumpIfFalse(0), cond.span.start);
                self.loops.push(LoopCx {
                    continue_to: Some(start),
                    continues: Vec::new(),
                    breaks: Vec::new(),
                });
                self.block(body);
                self.emit(Op::Jump(start), at);
                self.end_loop(jf);
            }
            StmtKind::For { var, iter, body } => self.for_loop(var, iter, body, at),
            StmtKind::Break => {
                let j = self.emit(Op::Jump(0), at);
                self.loops
                    .last_mut()
                    .expect("break outside loop")
                    .breaks
                    .push(j);
            }
            StmtKind::Continue => {
                let cx = self.loops.last().expect("continue outside loop");
                match cx.continue_to {
                    Some(target) => {
                        self.emit(Op::Jump(target), at);
                    }
                    None => {
                        let j = self.emit(Op::Jump(0), at);
                        self.loops.last_mut().expect("loop").continues.push(j);
                    }
                }
            }
            StmtKind::Block(b) => self.block(b),
            StmtKind::Expr(e) => {
                self.expr(e);
                self.emit(Op::Pop, at);
            }
        }
    }

    /// Patch the loop exit and every `break` to the current position.
    fn end_loop(&mut self, exit_jump: usize) {
        let cx = self.loops.pop().expect("loop context");
        self.patch(exit_jump);
        for b in cx.breaks {
            self.patch(b);
        }
    }

    /// `for` compiles to a counted loop over hidden locals:
    ///
    /// ```text
    ///     <init: counter = start, limit = end | seq = array, counter = 0>
    /// top: counter < limit (or seq.len()) ?  else -> exit
    ///     var = counter (or seq[counter])
    ///     <body>
    /// step: counter = counter + 1; jump top
    /// exit:
    /// ```
    fn for_loop(&mut self, var: &Ident, iter: &ForIter, body: &Block, at: usize) {
        let counter = self.hidden();
        let seq_or_limit = self.hidden();
        let is_range = matches!(iter, ForIter::Range(..));
        match iter {
            ForIter::Range(start, end) => {
                self.expr(start);
                self.emit(Op::Store(counter), at);
                self.expr(end);
                self.emit(Op::Store(seq_or_limit), at);
            }
            ForIter::Each(e) => {
                self.expr(e);
                if matches!(self.analysis.types.get(&e.id), Some(Type::Map(..))) {
                    self.emit(Op::Method(Method::Keys.id()), e.span.start);
                }
                self.emit(Op::Store(seq_or_limit), at);
                self.push_int(0, at);
                self.emit(Op::Store(counter), at);
            }
        }
        let top = self.here();
        self.emit(Op::Load(counter), at);
        self.emit(Op::Load(seq_or_limit), at);
        if !is_range {
            self.emit(Op::Method(Method::Len.id()), at);
        }
        self.emit(Op::Lt, at);
        let exit = self.emit(Op::JumpIfFalse(0), at);

        self.scopes.push(HashMap::new());
        let var_slot = self.declare(&var.name);
        self.emit(Op::Load(if is_range { counter } else { seq_or_limit }), at);
        if !is_range {
            self.emit(Op::Load(counter), at);
            self.emit(Op::Index, at);
        }
        self.emit(Op::Store(var_slot), var.span.start);
        self.loops.push(LoopCx {
            continue_to: None,
            continues: Vec::new(),
            breaks: Vec::new(),
        });
        self.block(body);
        self.scopes.pop();

        let continues = std::mem::take(&mut self.loops.last_mut().expect("loop").continues);
        for c in continues {
            self.patch(c);
        }
        self.emit(Op::Load(counter), at);
        self.push_int(1, at);
        self.emit(Op::Add, at);
        self.emit(Op::Store(counter), at);
        self.emit(Op::Jump(top), at);
        self.end_loop(exit);
    }

    fn expr(&mut self, e: &Expr) {
        let at = e.span.start;
        match &e.kind {
            ExprKind::Int(v) => self.push_int(*v, at),
            ExprKind::Float(v) => {
                let i = self.consts.float(*v);
                self.emit(Op::PushConst(i), at);
            }
            ExprKind::Bool(true) => {
                self.emit(Op::PushTrue, at);
            }
            ExprKind::Bool(false) => {
                self.emit(Op::PushFalse, at);
            }
            ExprKind::Str(s) => {
                let i = self.consts.str(s);
                self.emit(Op::PushConst(i), at);
            }
            ExprKind::Interp(parts) => {
                for part in parts {
                    match part {
                        InterpPart::Lit(s) => {
                            let i = self.consts.str(s);
                            self.emit(Op::PushConst(i), at);
                        }
                        InterpPart::Expr(inner) => self.expr(inner),
                    }
                }
                self.emit(Op::Format(parts.len() as u32), at);
            }
            ExprKind::Array(items) => {
                for item in items {
                    self.expr(item);
                }
                self.emit(Op::MakeArray(items.len() as u32), at);
            }
            ExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.expr(k);
                    self.expr(v);
                }
                self.emit(Op::MakeMap(entries.len() as u32), at);
            }
            ExprKind::Index { base, index } => {
                self.expr(base);
                self.expr(index);
                self.emit(Op::Index, index.span.start);
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                let m = *self
                    .analysis
                    .methods
                    .get(&e.id)
                    .expect("method resolved by sema");
                if m.mutating() {
                    let ExprKind::Var(name) = &receiver.kind else {
                        unreachable!("sema requires a variable receiver for mutating methods")
                    };
                    let slot = self.slot(name);
                    for a in args {
                        self.expr(a);
                    }
                    self.emit(Op::MethodMut(slot, m.id()), method.span.start);
                } else {
                    self.expr(receiver);
                    for a in args {
                        self.expr(a);
                    }
                    self.emit(Op::Method(m.id()), method.span.start);
                }
            }
            ExprKind::Var(name) => {
                let slot = self.slot(name);
                self.emit(Op::Load(slot), at);
            }
            ExprKind::Unary { op, expr } => {
                self.expr(expr);
                let op = match op {
                    UnOp::Neg => Op::Neg,
                    UnOp::Not => Op::Not,
                };
                self.emit(op, at);
            }
            ExprKind::Binary {
                op: BinOp::And,
                op_span,
                lhs,
                rhs,
            } => {
                self.expr(lhs);
                let jf = self.emit(Op::JumpIfFalse(0), op_span.start);
                self.expr(rhs);
                let jend = self.emit(Op::Jump(0), op_span.start);
                self.patch(jf);
                self.emit(Op::PushFalse, op_span.start);
                self.patch(jend);
            }
            ExprKind::Binary {
                op: BinOp::Or,
                op_span,
                lhs,
                rhs,
            } => {
                self.expr(lhs);
                let jf = self.emit(Op::JumpIfFalse(0), op_span.start);
                self.emit(Op::PushTrue, op_span.start);
                let jend = self.emit(Op::Jump(0), op_span.start);
                self.patch(jf);
                self.expr(rhs);
                self.patch(jend);
            }
            ExprKind::Binary {
                op,
                op_span,
                lhs,
                rhs,
            } => {
                self.expr(lhs);
                self.expr(rhs);
                let is_str = self.analysis.types.get(&e.id) == Some(&Type::Str);
                let op = match op {
                    BinOp::Add if is_str => Op::Concat,
                    BinOp::Add => Op::Add,
                    BinOp::Sub => Op::Sub,
                    BinOp::Mul => Op::Mul,
                    BinOp::Div => Op::Div,
                    BinOp::Rem => Op::Rem,
                    BinOp::Eq => Op::Eq,
                    BinOp::Ne => Op::Ne,
                    BinOp::Lt => Op::Lt,
                    BinOp::Le => Op::Le,
                    BinOp::Gt => Op::Gt,
                    BinOp::Ge => Op::Ge,
                    BinOp::And | BinOp::Or => unreachable!("handled above"),
                };
                self.emit(op, op_span.start);
            }
            ExprKind::Call { callee, args } => {
                for a in args {
                    self.expr(a);
                }
                if let Some(b) = Builtin::by_name(&callee.name) {
                    self.emit(Op::Builtin(b.id()), callee.span.start);
                    return;
                }
                let index = *self
                    .fn_index
                    .get(&callee.name)
                    .unwrap_or_else(|| panic!("unresolved function `{}`", callee.name));
                self.emit(Op::Call(index), callee.span.start);
            }
        }
    }
}
