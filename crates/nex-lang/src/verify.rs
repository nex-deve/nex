//! Bytecode verifier.
//!
//! Every module is verified before it runs, whether it came from the
//! compiler or from a `.nexdeve` file. A verified function:
//! - only references constants, locals, functions and jump targets that exist;
//! - only calls functions of kind `Fn` (apps are entry points, not callable);
//! - never underflows its operand stack and stays under `MAX_STACK_DEPTH`;
//! - has the same stack depth on every path into each instruction;
//! - executes `Return` with exactly one value on the stack;
//! - cannot fall off the end of its code.
//!
//! The verifier does not check value types; the VM checks those at run time
//! and reports a runtime error (never a crash) on a mismatch.

use crate::bytecode::{Builtin, FnKind, Function, Method, Module, Op};
use std::fmt;

pub const MAX_STACK_DEPTH: u32 = 65_536;
pub const MAX_LOCALS: u32 = 65_535;
/// Maximum operand of `Format`, `MakeArray` and `MakeMap`.
pub const MAX_ITEMS: u32 = 30_000;
/// Maximum index-path depth of `StoreIndex`.
pub const MAX_STORE_DEPTH: u32 = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError {
    pub function: Option<String>,
    pub pc: Option<usize>,
    pub message: String,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.function, self.pc) {
            (Some(func), Some(pc)) => {
                write!(f, "in `{func}` at instruction {pc}: {}", self.message)
            }
            (Some(func), None) => write!(f, "in `{func}`: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

pub fn verify(m: &Module) -> Result<(), VerifyError> {
    let module_err = |message: String| VerifyError {
        function: None,
        pc: None,
        message,
    };
    for &a in &m.apps {
        match m.functions.get(a as usize) {
            None => return Err(module_err(format!("app index {a} is out of range"))),
            Some(f) if f.kind != FnKind::App => {
                return Err(module_err(format!(
                    "entry {a} (`{}`) is not an app",
                    f.name
                )))
            }
            Some(_) => {}
        }
    }
    for f in &m.functions {
        verify_fn(m, f)?;
    }
    Ok(())
}

fn stack_effect(m: &Module, op: Op) -> (u32, u32) {
    match op {
        Op::PushConst(_) | Op::PushUnit | Op::PushTrue | Op::PushFalse | Op::Load(_) => (0, 1),
        Op::Store(_) | Op::Pop | Op::Say | Op::JumpIfFalse(_) => (1, 0),
        Op::Add
        | Op::Sub
        | Op::Mul
        | Op::Div
        | Op::Rem
        | Op::Eq
        | Op::Ne
        | Op::Lt
        | Op::Le
        | Op::Gt
        | Op::Ge
        | Op::Concat => (2, 1),
        Op::Neg | Op::Not => (1, 1),
        Op::Format(n) | Op::MakeArray(n) => (n, 1),
        Op::MakeMap(n) => (2 * n, 1),
        Op::Index => (2, 1),
        Op::StoreIndex(_, d) => (d + 1, 0),
        Op::Jump(_) | Op::TryBegin(_) | Op::TryEnd => (0, 0),
        Op::Call(i) => (m.functions[i as usize].arity, 1),
        Op::Return | Op::Fail => (1, 0),
        Op::Method(id) => (1 + Method::from_id(id).expect("checked").argc(), 1),
        Op::MethodMut(_, id) => (Method::from_id(id).expect("checked").argc(), 1),
        Op::Builtin(id) => (Builtin::from_id(id).expect("checked").argc(), 1),
    }
}

fn verify_fn(m: &Module, f: &Function) -> Result<(), VerifyError> {
    let err = |pc: Option<usize>, message: String| VerifyError {
        function: Some(f.name.clone()),
        pc,
        message,
    };
    let n = f.code.len();
    if n == 0 {
        return Err(err(None, "function has no code".into()));
    }
    if f.positions.len() != n {
        return Err(err(
            None,
            "position table length does not match code length".into(),
        ));
    }
    if f.num_locals > MAX_LOCALS {
        return Err(err(None, format!("too many locals ({})", f.num_locals)));
    }
    if f.arity > f.num_locals {
        return Err(err(None, "arity exceeds number of locals".into()));
    }
    if f.kind == FnKind::App && f.arity != 0 {
        return Err(err(None, "an app cannot take parameters".into()));
    }
    if f.source as usize >= m.sources.len() {
        return Err(err(
            None,
            format!("source index {} is out of range", f.source),
        ));
    }

    // Operand checks.
    for (pc, &op) in f.code.iter().enumerate() {
        let bad = match op {
            Op::PushConst(i) if i as usize >= m.consts.len() => {
                Some(format!("constant index {i} is out of range"))
            }
            Op::Load(i) | Op::Store(i) if i >= f.num_locals => {
                Some(format!("local slot {i} is out of range"))
            }
            Op::Jump(t) | Op::JumpIfFalse(t) | Op::TryBegin(t) if t as usize >= n => {
                Some(format!("jump target {t} is out of range"))
            }
            Op::Call(i) => match m.functions.get(i as usize) {
                None => Some(format!("function index {i} is out of range")),
                Some(callee) if callee.kind != FnKind::Fn => {
                    Some(format!("`{}` is an app and cannot be called", callee.name))
                }
                Some(_) => None,
            },
            Op::Format(k) if k == 0 || k > MAX_ITEMS => Some(format!("invalid Format operand {k}")),
            Op::MakeArray(k) | Op::MakeMap(k) if k > MAX_ITEMS => {
                Some(format!("collection literal has too many items ({k})"))
            }
            Op::StoreIndex(slot, _) if slot >= f.num_locals => {
                Some(format!("local slot {slot} is out of range"))
            }
            Op::StoreIndex(_, depth) if depth == 0 || depth > MAX_STORE_DEPTH => {
                Some(format!("invalid StoreIndex depth {depth}"))
            }
            Op::Method(id) => match Method::from_id(id) {
                None => Some(format!("unknown method id {id}")),
                Some(mt) if mt.mutating() => {
                    Some(format!("method `{}` must use MethodMut", mt.name()))
                }
                Some(_) => None,
            },
            Op::MethodMut(slot, id) => match Method::from_id(id) {
                None => Some(format!("unknown method id {id}")),
                Some(mt) if !mt.mutating() => {
                    Some(format!("method `{}` must use Method", mt.name()))
                }
                Some(_) if slot >= f.num_locals => {
                    Some(format!("local slot {slot} is out of range"))
                }
                Some(_) => None,
            },
            Op::Builtin(id) if Builtin::from_id(id).is_none() => {
                Some(format!("unknown built-in id {id}"))
            }
            _ => None,
        };
        if let Some(message) = bad {
            return Err(err(Some(pc), message));
        }
    }

    // Stack-depth data flow over reachable instructions.
    let mut depth: Vec<Option<u32>> = vec![None; n];
    depth[0] = Some(0);
    let mut work = vec![0usize];
    while let Some(pc) = work.pop() {
        let d = depth[pc].expect("queued instruction has a depth");
        let op = f.code[pc];
        let (pops, pushes) = stack_effect(m, op);
        if d < pops {
            return Err(err(
                Some(pc),
                format!(
                    "stack underflow: {} needs {pops} value(s), stack has {d}",
                    op.name()
                ),
            ));
        }
        let next_depth = d - pops + pushes;
        if next_depth > MAX_STACK_DEPTH {
            return Err(err(Some(pc), "operand stack too deep".into()));
        }
        // (successor, stack depth on entry to it)
        let successors: Vec<(usize, u32)> = match op {
            Op::Return => {
                if d != 1 {
                    return Err(err(
                        Some(pc),
                        format!("Return needs exactly 1 value on the stack, found {d}"),
                    ));
                }
                vec![]
            }
            Op::Fail => vec![],
            Op::Jump(t) => vec![(t as usize, next_depth)],
            Op::JumpIfFalse(t) => vec![(pc + 1, next_depth), (t as usize, next_depth)],
            // The handler starts with the stack as it was here, plus the
            // error message.
            Op::TryBegin(t) => vec![(pc + 1, next_depth), (t as usize, next_depth + 1)],
            _ => vec![(pc + 1, next_depth)],
        };
        for (s, next_depth) in successors {
            if s >= n {
                return Err(err(
                    Some(pc),
                    "execution can fall off the end of the function".into(),
                ));
            }
            match depth[s] {
                None => {
                    depth[s] = Some(next_depth);
                    work.push(s);
                }
                Some(existing) if existing != next_depth => {
                    return Err(err(
                        Some(s),
                        format!("inconsistent stack depth: {existing} on one path, {next_depth} on another"),
                    ));
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Const, Pos};

    fn module(code: Vec<Op>, num_locals: u32) -> Module {
        let positions = vec![Pos::default(); code.len()];
        Module {
            sources: vec!["t".into()],
            consts: vec![Const::Int(1)],
            functions: vec![Function {
                name: "main".into(),
                kind: FnKind::App,
                arity: 0,
                num_locals,
                code,
                positions,
                source: 0,
            }],
            apps: vec![0],
        }
    }

    fn check(code: Vec<Op>) -> Result<(), String> {
        verify(&module(code, 1)).map_err(|e| e.to_string())
    }

    #[test]
    fn accepts_simple_code() {
        assert_eq!(
            check(vec![Op::PushConst(0), Op::Say, Op::PushUnit, Op::Return]),
            Ok(())
        );
        // try { fail } catch e { say e }
        assert_eq!(
            check(vec![
                Op::TryBegin(5),
                Op::PushConst(0),
                Op::Fail,
                Op::TryEnd,
                Op::Jump(7),
                Op::Store(0),
                Op::Jump(7),
                Op::PushUnit,
                Op::Return
            ]),
            Ok(())
        );
        // if/else merging at equal depth
        assert_eq!(
            check(vec![
                Op::PushTrue,
                Op::JumpIfFalse(4),
                Op::PushConst(0),
                Op::Jump(5),
                Op::PushConst(0),
                Op::Say,
                Op::PushUnit,
                Op::Return
            ]),
            Ok(())
        );
    }

    #[test]
    fn rejects_bad_code() {
        let cases: Vec<(Vec<Op>, &str)> = vec![
            (vec![], "no code"),
            (vec![Op::Pop, Op::PushUnit, Op::Return], "underflow"),
            (vec![Op::PushConst(9), Op::Return], "constant index"),
            (vec![Op::Load(5), Op::Return], "local slot"),
            (vec![Op::Jump(99)], "jump target"),
            (vec![Op::Call(0), Op::Return], "cannot be called"),
            (vec![Op::Call(3), Op::Return], "function index"),
            (vec![Op::PushUnit], "fall off the end"),
            (
                vec![Op::PushUnit, Op::PushUnit, Op::Return],
                "exactly 1 value",
            ),
            (vec![Op::Format(0), Op::Return], "Format operand"),
            (vec![Op::Method(99), Op::Return], "unknown method"),
            (vec![Op::Method(32), Op::Return], "must use MethodMut"),
            (vec![Op::MethodMut(0, 0), Op::Return], "must use Method"),
            (vec![Op::MethodMut(7, 32), Op::Return], "local slot"),
            (
                vec![Op::StoreIndex(0, 0), Op::PushUnit, Op::Return],
                "StoreIndex depth",
            ),
            (vec![Op::Builtin(99), Op::Return], "unknown built-in"),
            (vec![Op::MakeArray(40_000), Op::Return], "too many items"),
            (
                vec![Op::TryBegin(50), Op::PushUnit, Op::Return],
                "jump target",
            ),
            // The handler is entered with one more value than TryBegin had.
            (
                vec![
                    Op::TryBegin(3),
                    Op::PushUnit,
                    Op::Return,
                    Op::PushUnit,
                    Op::Return,
                ],
                "exactly 1 value",
            ),
            (vec![Op::PushUnit, Op::Index, Op::Return], "underflow"),
            (vec![Op::PushUnit, Op::MakeMap(1), Op::Return], "underflow"),
            (
                vec![
                    Op::PushTrue,
                    Op::JumpIfFalse(3),
                    Op::PushUnit,
                    Op::PushUnit,
                    Op::Return,
                ],
                "inconsistent stack depth",
            ),
        ];
        for (code, want) in cases {
            let got = check(code.clone()).expect_err(&format!("{code:?} should fail"));
            assert!(got.contains(want), "{code:?}: {got}");
        }
    }

    #[test]
    fn rejects_unbounded_stack_growth_and_bad_tables() {
        // A loop that pushes forever has inconsistent depth at the loop head.
        assert!(check(vec![Op::PushUnit, Op::Jump(0)]).is_err());
        let mut m = module(vec![Op::PushUnit, Op::Return], 0);
        m.apps = vec![4];
        assert!(verify(&m).is_err());
        let mut m = module(vec![Op::PushUnit, Op::Return], 0);
        m.functions[0].positions.pop();
        assert!(verify(&m).is_err());
    }
}
