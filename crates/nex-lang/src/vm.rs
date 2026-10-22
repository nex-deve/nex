//! Stack-based bytecode virtual machine.
//!
//! The VM is not recursive: NEX calls push frames onto a heap-allocated
//! frame stack, so deep NEX recursion cannot overflow the native stack.
//! Every resource the program controls is bounded by [`Limits`].
//!
//! Arrays and maps have value semantics. They are shared behind `Rc` and
//! copied on write (`Rc::make_mut`), so assigning a collection to another
//! variable never creates aliasing, and in-place updates of an unshared
//! local (`xs.push(..)`, `xs[i] = ..`) do not copy.

use crate::bytecode::{Builtin, Const, FnKind, Method, Module, Op, Pos};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::io::Write;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Rc<str>),
    Array(Rc<Vec<Value>>),
    Map(Rc<BTreeMap<Key, Value>>),
    Unit,
}

/// A map key. Maps iterate in key order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Bool(bool),
    Int(i64),
    Str(Rc<str>),
}

impl Key {
    fn from_value(v: Value) -> Result<Key, String> {
        match v {
            Value::Bool(b) => Ok(Key::Bool(b)),
            Value::Int(i) => Ok(Key::Int(i)),
            Value::Str(s) => Ok(Key::Str(s)),
            other => Err(format!(
                "{} cannot be a map key (invalid bytecode)",
                other.kind()
            )),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Key::Bool(b) => Value::Bool(*b),
            Key::Int(i) => Value::Int(*i),
            Key::Str(s) => Value::Str(s.clone()),
        }
    }
}

/// Text form of a float: always shows a decimal point for whole numbers.
fn fmt_float(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e16 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Bool(_) => "Bool",
            Value::Str(_) => "Str",
            Value::Array(_) => "array",
            Value::Map(_) => "map",
            Value::Unit => "Unit",
        }
    }

    /// Text form inside a collection: strings are quoted.
    fn write_nested(&self, f: &mut impl fmt::Write) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "{:?}", &**s),
            other => write!(f, "{other}"),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(v) => write!(f, "{v}"),
            Value::Float(v) => f.write_str(&fmt_float(*v)),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => f.write_str(s),
            Value::Unit => f.write_str("()"),
            Value::Array(items) => {
                f.write_str("[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    v.write_nested(f)?;
                }
                f.write_str("]")
            }
            Value::Map(entries) => {
                f.write_str("{")?;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    k.to_value().write_nested(f)?;
                    f.write_str(": ")?;
                    v.write_nested(f)?;
                }
                f.write_str("}")
            }
        }
    }
}

/// Resource limits for one VM run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum instructions executed; `None` means unlimited.
    pub max_steps: Option<u64>,
    /// Maximum number of active call frames (including the app frame).
    pub max_call_depth: usize,
    /// Maximum values on the shared value stack (locals + operands).
    pub max_stack_values: usize,
    /// Maximum length in bytes of any string the program builds.
    pub max_string_bytes: usize,
    /// Maximum number of elements in any array or map.
    pub max_collection_len: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_steps: None,
            max_call_depth: 1_000,
            max_stack_values: 1_000_000,
            max_string_bytes: 16 * 1024 * 1024,
            max_collection_len: 10_000_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    pub function: String,
    pub kind: FnKind,
    pub pos: Pos,
}

/// A runtime failure. `trace[0]` is the innermost frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
    pub trace: Vec<TraceEntry>,
}

impl RuntimeError {
    /// Render as `runtime error: ...` plus a call trace.
    pub fn render(&self, source_name: &str) -> String {
        let mut out = format!("runtime error: {}\n", self.message);
        for t in &self.trace {
            let kind = match t.kind {
                FnKind::Fn => "fn",
                FnKind::App => "app",
            };
            let _ = writeln!(
                out,
                "  at {kind} {} ({source_name}:{}:{})",
                t.function, t.pos.line, t.pos.col
            );
        }
        out
    }
}

struct Frame {
    func: usize,
    pc: usize,
    base: usize,
}

/// Run one app of a module. The module must have passed
/// [`crate::verify::verify`]; unverified bytecode may produce runtime
/// errors but never undefined behaviour.
pub fn run_app(
    module: &Module,
    app: u32,
    out: &mut dyn Write,
    limits: &Limits,
) -> Result<(), RuntimeError> {
    let consts = module
        .consts
        .iter()
        .map(|c| match c {
            Const::Int(v) => Value::Int(*v),
            Const::Float(v) => Value::Float(*v),
            Const::Str(s) => Value::Str(Rc::from(s.as_str())),
        })
        .collect();
    let mut vm = Vm {
        m: module,
        consts,
        out,
        limits,
        stack: Vec::new(),
        frames: Vec::new(),
        steps: 0,
    };
    match module.functions.get(app as usize) {
        Some(f) if f.kind == FnKind::App => {}
        _ => return Err(vm.error(format!("no app with index {app}"))),
    }
    vm.enter(app as usize)?;
    vm.run()
}

struct Vm<'a> {
    m: &'a Module,
    consts: Vec<Value>,
    out: &'a mut dyn Write,
    limits: &'a Limits,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    steps: u64,
}

/// Result of a helper that does not have access to the VM; the VM turns
/// the message into a [`RuntimeError`] with a trace.
type Res<T> = Result<T, String>;

fn want_int(v: &Value) -> Res<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        other => Err(format!(
            "expected Int, found {} (invalid bytecode)",
            other.kind()
        )),
    }
}

fn want_str(v: &Value) -> Res<Rc<str>> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => Err(format!(
            "expected Str, found {} (invalid bytecode)",
            other.kind()
        )),
    }
}

/// Convert an Int index into a position in `0..len` (or `0..=len` when
/// `allow_end`), with a readable error.
fn index_in(i: i64, len: usize, allow_end: bool) -> Res<usize> {
    let max = if allow_end {
        len
    } else {
        len.saturating_sub(1)
    };
    if i < 0 || (i as u64) > max as u64 || (!allow_end && len == 0) {
        return Err(format!("index {i} is out of bounds for length {len}"));
    }
    Ok(i as usize)
}

fn slice_range(a: i64, b: i64, len: usize) -> Res<(usize, usize)> {
    if a < 0 || b < a || b as u64 > len as u64 {
        return Err(format!("slice({a}, {b}) is out of bounds for length {len}"));
    }
    Ok((a as usize, b as usize))
}

fn compare(a: &Value, b: &Value) -> Res<Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => Ok(x.total_cmp(y)),
        (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
        _ => Err(format!(
            "cannot compare {} with {} (invalid bytecode)",
            a.kind(),
            b.kind()
        )),
    }
}

fn char_index(s: &str, byte: usize) -> i64 {
    s[..byte].chars().count() as i64
}

impl Vm<'_> {
    fn error(&self, message: impl Into<String>) -> RuntimeError {
        let trace = self
            .frames
            .iter()
            .rev()
            .map(|fr| {
                let f = &self.m.functions[fr.func];
                let pc = fr.pc.saturating_sub(1);
                TraceEntry {
                    function: f.name.clone(),
                    kind: f.kind,
                    pos: f.positions.get(pc).copied().unwrap_or_default(),
                }
            })
            .collect();
        RuntimeError {
            message: message.into(),
            trace,
        }
    }

    fn push(&mut self, v: Value) -> Result<(), RuntimeError> {
        if self.stack.len() >= self.limits.max_stack_values {
            return Err(self.error("value stack limit exceeded"));
        }
        self.stack.push(v);
        Ok(())
    }

    /// Lowest stack index the current frame's operands may use.
    fn floor(&self) -> usize {
        self.frames.last().map_or(0, |fr| {
            fr.base + self.m.functions[fr.func].num_locals as usize
        })
    }

    fn pop(&mut self) -> Result<Value, RuntimeError> {
        if self.stack.len() <= self.floor() {
            return Err(self.error("operand stack underflow (invalid bytecode)"));
        }
        Ok(self.stack.pop().expect("checked above"))
    }

    /// Pop `n` values, returned in push order.
    fn pop_n(&mut self, n: usize) -> Result<Vec<Value>, RuntimeError> {
        if self.stack.len() < self.floor() + n {
            return Err(self.error("operand stack underflow (invalid bytecode)"));
        }
        Ok(self.stack.split_off(self.stack.len() - n))
    }

    fn pop_bool(&mut self) -> Result<bool, RuntimeError> {
        match self.pop()? {
            Value::Bool(b) => Ok(b),
            other => Err(self.error(format!(
                "expected Bool, found {} (invalid bytecode)",
                other.kind()
            ))),
        }
    }

    fn check_string(&self, len: usize) -> Res<()> {
        if len > self.limits.max_string_bytes {
            return Err(format!(
                "string length limit of {} bytes exceeded",
                self.limits.max_string_bytes
            ));
        }
        Ok(())
    }

    fn check_len(&self, len: usize) -> Res<()> {
        if len > self.limits.max_collection_len {
            return Err(format!(
                "collection size limit of {} elements exceeded",
                self.limits.max_collection_len
            ));
        }
        Ok(())
    }

    fn str_value(&self, s: String) -> Res<Value> {
        self.check_string(s.len())?;
        Ok(Value::Str(Rc::from(s)))
    }

    fn enter(&mut self, func: usize) -> Result<(), RuntimeError> {
        let Some(f) = self.m.functions.get(func) else {
            return Err(self.error(format!("function index {func} out of range")));
        };
        if self.frames.len() >= self.limits.max_call_depth {
            return Err(self.error(format!(
                "call depth limit of {} exceeded (is there infinite recursion?)",
                self.limits.max_call_depth
            )));
        }
        let arity = f.arity as usize;
        if self.stack.len() < self.floor() + arity {
            return Err(self.error("not enough arguments on the stack (invalid bytecode)"));
        }
        let extra = f.num_locals.saturating_sub(f.arity) as usize;
        if self.stack.len() + extra > self.limits.max_stack_values {
            return Err(self.error("value stack limit exceeded"));
        }
        let base = self.stack.len() - arity;
        self.stack.extend(std::iter::repeat_n(Value::Unit, extra));
        self.frames.push(Frame { func, pc: 0, base });
        Ok(())
    }

    fn arith(&self, op: Op, a: Value, b: Value) -> Res<Value> {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => {
                let (r, sym) = match op {
                    Op::Add => (a.checked_add(b), "+"),
                    Op::Sub => (a.checked_sub(b), "-"),
                    Op::Mul => (a.checked_mul(b), "*"),
                    Op::Div if b == 0 => return Err("division by zero".into()),
                    Op::Div => (a.checked_div(b), "/"),
                    Op::Rem if b == 0 => return Err("remainder by zero".into()),
                    _ => (a.checked_rem(b), "%"),
                };
                r.map(Value::Int)
                    .ok_or_else(|| format!("integer overflow in `{a} {sym} {b}`"))
            }
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(match op {
                Op::Add => a + b,
                Op::Sub => a - b,
                Op::Mul => a * b,
                Op::Div => a / b,
                _ => a % b,
            })),
            (Value::Array(a), Value::Array(b)) if op == Op::Add => {
                self.check_len(a.len() + b.len())?;
                let mut v = Vec::with_capacity(a.len() + b.len());
                v.extend(a.iter().cloned());
                v.extend(b.iter().cloned());
                Ok(Value::Array(Rc::new(v)))
            }
            (a, b) => Err(format!(
                "cannot apply {} to {} and {} (invalid bytecode)",
                op.name(),
                a.kind(),
                b.kind()
            )),
        }
    }

    fn index(&self, base: Value, index: Value) -> Res<Value> {
        match base {
            Value::Array(items) => {
                let i = index_in(want_int(&index)?, items.len(), false)?;
                Ok(items[i].clone())
            }
            Value::Map(map) => {
                let key = Key::from_value(index)?;
                map.get(&key).cloned().ok_or_else(|| {
                    let mut shown = String::new();
                    let _ = key.to_value().write_nested(&mut shown);
                    format!("key {shown} not found in map (use .get(key, default) or .has(key))")
                })
            }
            other => Err(format!(
                "cannot index into {} (invalid bytecode)",
                other.kind()
            )),
        }
    }

    fn store_path(&self, target: &mut Value, path: &[Value], value: Value) -> Res<()> {
        let (first, rest) = path.split_first().expect("non-empty path");
        match target {
            Value::Array(items) => {
                let items = Rc::make_mut(items);
                let i = index_in(want_int(first)?, items.len(), false)?;
                if rest.is_empty() {
                    items[i] = value;
                    Ok(())
                } else {
                    self.store_path(&mut items[i], rest, value)
                }
            }
            Value::Map(map) => {
                let key = Key::from_value(first.clone())?;
                if rest.is_empty() {
                    if !map.contains_key(&key) {
                        self.check_len(map.len() + 1)?;
                    }
                    Rc::make_mut(map).insert(key, value);
                    return Ok(());
                }
                let map = Rc::make_mut(map);
                match map.get_mut(&key) {
                    Some(inner) => self.store_path(inner, rest, value),
                    None => {
                        let mut shown = String::new();
                        let _ = key.to_value().write_nested(&mut shown);
                        Err(format!("key {shown} not found in map"))
                    }
                }
            }
            other => Err(format!(
                "cannot index into {} (invalid bytecode)",
                other.kind()
            )),
        }
    }

    fn method(&self, m: Method, recv: Value, args: &[Value]) -> Res<Value> {
        let arg = |i: usize| {
            args.get(i)
                .cloned()
                .ok_or("missing argument (invalid bytecode)")
        };
        match (m, recv) {
            (Method::Len, Value::Array(a)) => Ok(Value::Int(a.len() as i64)),
            (Method::Len, Value::Map(map)) => Ok(Value::Int(map.len() as i64)),
            (Method::Len, Value::Str(s)) => Ok(Value::Int(s.chars().count() as i64)),
            (Method::Contains, Value::Array(a)) => {
                let x = arg(0)?;
                Ok(Value::Bool(a.contains(&x)))
            }
            (Method::Contains, Value::Str(s)) => Ok(Value::Bool(s.contains(&*want_str(&arg(0)?)?))),
            (Method::IndexOf, Value::Array(a)) => {
                let x = arg(0)?;
                Ok(Value::Int(
                    a.iter().position(|v| *v == x).map_or(-1, |i| i as i64),
                ))
            }
            (Method::IndexOf, Value::Str(s)) => {
                let sub = want_str(&arg(0)?)?;
                Ok(Value::Int(s.find(&*sub).map_or(-1, |b| char_index(&s, b))))
            }
            (Method::Join, Value::Array(a)) => {
                let sep = want_str(&arg(0)?)?;
                let mut out = String::new();
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push_str(&sep);
                    }
                    let _ = write!(out, "{v}");
                    self.check_string(out.len())?;
                }
                self.str_value(out)
            }
            (Method::Sorted, Value::Array(a)) => {
                let mut v = (*a).clone();
                let mut failure = None;
                v.sort_by(|x, y| {
                    compare(x, y).unwrap_or_else(|e| {
                        failure = Some(e);
                        Ordering::Equal
                    })
                });
                match failure {
                    Some(e) => Err(e),
                    None => Ok(Value::Array(Rc::new(v))),
                }
            }
            (Method::Reversed, Value::Array(a)) => {
                let mut v = (*a).clone();
                v.reverse();
                Ok(Value::Array(Rc::new(v)))
            }
            (Method::Slice, Value::Array(a)) => {
                let (s, e) = slice_range(want_int(&arg(0)?)?, want_int(&arg(1)?)?, a.len())?;
                Ok(Value::Array(Rc::new(a[s..e].to_vec())))
            }
            (Method::Slice, Value::Str(st)) => {
                let len = st.chars().count();
                let (s, e) = slice_range(want_int(&arg(0)?)?, want_int(&arg(1)?)?, len)?;
                self.str_value(st.chars().skip(s).take(e - s).collect())
            }
            (Method::Upper, Value::Str(s)) => self.str_value(s.to_uppercase()),
            (Method::Lower, Value::Str(s)) => self.str_value(s.to_lowercase()),
            (Method::Trim, Value::Str(s)) => self.str_value(s.trim().to_string()),
            (Method::StartsWith, Value::Str(s)) => {
                Ok(Value::Bool(s.starts_with(&*want_str(&arg(0)?)?)))
            }
            (Method::EndsWith, Value::Str(s)) => {
                Ok(Value::Bool(s.ends_with(&*want_str(&arg(0)?)?)))
            }
            (Method::Split, Value::Str(s)) => {
                let sep = want_str(&arg(0)?)?;
                if sep.is_empty() {
                    return Err("split separator must not be empty (use .chars())".into());
                }
                let parts: Vec<Value> = s.split(&*sep).map(|p| Value::Str(Rc::from(p))).collect();
                self.check_len(parts.len())?;
                Ok(Value::Array(Rc::new(parts)))
            }
            (Method::Chars, Value::Str(s)) => {
                let chars: Vec<Value> = s
                    .chars()
                    .map(|c| Value::Str(Rc::from(c.to_string())))
                    .collect();
                self.check_len(chars.len())?;
                Ok(Value::Array(Rc::new(chars)))
            }
            (Method::Replace, Value::Str(s)) => {
                let from = want_str(&arg(0)?)?;
                let to = want_str(&arg(1)?)?;
                if from.is_empty() {
                    return Err("replace: the text to find must not be empty".into());
                }
                let count = s.matches(&*from).count();
                let new_len = s.len() - count * from.len() + count * to.len();
                self.check_string(new_len)?;
                self.str_value(s.replace(&*from, &to))
            }
            (Method::Has, Value::Map(map)) => {
                Ok(Value::Bool(map.contains_key(&Key::from_value(arg(0)?)?)))
            }
            (Method::Get, Value::Map(map)) => {
                let key = Key::from_value(arg(0)?)?;
                Ok(map.get(&key).cloned().unwrap_or(arg(1)?))
            }
            (Method::Keys, Value::Map(map)) => Ok(Value::Array(Rc::new(
                map.keys().map(Key::to_value).collect(),
            ))),
            (Method::Values, Value::Map(map)) => {
                Ok(Value::Array(Rc::new(map.values().cloned().collect())))
            }
            (m, recv) => Err(format!(
                "{} has no method `{}` (invalid bytecode)",
                recv.kind(),
                m.name()
            )),
        }
    }

    fn method_mut(&self, m: Method, target: &mut Value, args: &[Value]) -> Res<Value> {
        let arg = |i: usize| {
            args.get(i)
                .cloned()
                .ok_or("missing argument (invalid bytecode)")
        };
        match (m, target) {
            (Method::Push, Value::Array(a)) => {
                self.check_len(a.len() + 1)?;
                Rc::make_mut(a).push(arg(0)?);
                Ok(Value::Unit)
            }
            (Method::Pop, Value::Array(a)) => Rc::make_mut(a)
                .pop()
                .ok_or_else(|| "pop() on an empty array".to_string()),
            (Method::Insert, Value::Array(a)) => {
                let i = index_in(want_int(&arg(0)?)?, a.len(), true)?;
                self.check_len(a.len() + 1)?;
                Rc::make_mut(a).insert(i, arg(1)?);
                Ok(Value::Unit)
            }
            (Method::RemoveAt, Value::Array(a)) => {
                let i = index_in(want_int(&arg(0)?)?, a.len(), false)?;
                Ok(Rc::make_mut(a).remove(i))
            }
            (Method::Clear, Value::Array(a)) => {
                Rc::make_mut(a).clear();
                Ok(Value::Unit)
            }
            (Method::Remove, Value::Map(map)) => {
                let key = Key::from_value(arg(0)?)?;
                Ok(Value::Bool(Rc::make_mut(map).remove(&key).is_some()))
            }
            (Method::Clear, Value::Map(map)) => {
                Rc::make_mut(map).clear();
                Ok(Value::Unit)
            }
            (m, target) => Err(format!(
                "{} has no method `{}` (invalid bytecode)",
                target.kind(),
                m.name()
            )),
        }
    }

    fn builtin(&self, b: Builtin, args: &[Value]) -> Res<Value> {
        let float = |i: usize| match args.get(i) {
            Some(Value::Float(x)) => Ok(*x),
            _ => Err("expected Float argument (invalid bytecode)".to_string()),
        };
        match b {
            Builtin::ToFloat => Ok(Value::Float(
                want_int(args.first().ok_or("missing argument")?)? as f64,
            )),
            Builtin::ToInt => {
                let x = float(0)?;
                // Int covers [-2^63, 2^63); both bounds are exact in f64.
                let min = i64::MIN as f64;
                if !x.is_finite() || x < min || x >= -min {
                    return Err(format!("to_int({}) is out of range for Int", fmt_float(x)));
                }
                Ok(Value::Int(x.trunc() as i64))
            }
            Builtin::Sqrt => Ok(Value::Float(float(0)?.sqrt())),
            Builtin::Floor => Ok(Value::Float(float(0)?.floor())),
            Builtin::Round => Ok(Value::Float(float(0)?.round())),
            Builtin::ParseInt => {
                let s = want_str(args.first().ok_or("missing argument")?)?;
                s.trim()
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| format!("parse_int: {:?} is not a valid Int", &*s))
            }
            Builtin::ParseFloat => {
                let s = want_str(args.first().ok_or("missing argument")?)?;
                match s.trim().parse::<f64>() {
                    Ok(x) if x.is_finite() => Ok(Value::Float(x)),
                    _ => Err(format!("parse_float: {:?} is not a valid Float", &*s)),
                }
            }
        }
    }

    fn run(&mut self) -> Result<(), RuntimeError> {
        let m = self.m;
        loop {
            let frame = self.frames.last_mut().expect("run with no frame");
            let (func, pc, base) = (frame.func, frame.pc, frame.base);
            let f = &m.functions[func];
            let Some(&op) = f.code.get(pc) else {
                return Err(
                    self.error("execution ran past the end of a function (invalid bytecode)")
                );
            };
            frame.pc = pc + 1;

            self.steps += 1;
            if let Some(max) = self.limits.max_steps {
                if self.steps > max {
                    return Err(self.error(format!("step limit of {max} instructions exceeded")));
                }
            }

            // Helpers return `Res`; attach the trace here.
            macro_rules! vm_try {
                ($e:expr) => {
                    match $e {
                        Ok(v) => v,
                        Err(msg) => return Err(self.error(msg)),
                    }
                };
            }

            match op {
                Op::PushConst(i) => {
                    let Some(v) = self.consts.get(i as usize).cloned() else {
                        return Err(self.error("constant index out of range (invalid bytecode)"));
                    };
                    self.push(v)?;
                }
                Op::PushUnit => self.push(Value::Unit)?,
                Op::PushTrue => self.push(Value::Bool(true))?,
                Op::PushFalse => self.push(Value::Bool(false))?,
                Op::Load(i) => {
                    if i >= f.num_locals {
                        return Err(self.error("local slot out of range (invalid bytecode)"));
                    }
                    let v = self.stack[base + i as usize].clone();
                    self.push(v)?;
                }
                Op::Store(i) => {
                    if i >= f.num_locals {
                        return Err(self.error("local slot out of range (invalid bytecode)"));
                    }
                    let v = self.pop()?;
                    self.stack[base + i as usize] = v;
                }
                Op::Pop => {
                    self.pop()?;
                }
                Op::MakeArray(n) => {
                    let items = self.pop_n(n as usize)?;
                    vm_try!(self.check_len(items.len()));
                    self.push(Value::Array(Rc::new(items)))?;
                }
                Op::MakeMap(n) => {
                    let flat = self.pop_n(2 * n as usize)?;
                    let mut map = BTreeMap::new();
                    let mut it = flat.into_iter();
                    while let (Some(k), Some(v)) = (it.next(), it.next()) {
                        map.insert(vm_try!(Key::from_value(k)), v);
                    }
                    self.push(Value::Map(Rc::new(map)))?;
                }
                Op::Index => {
                    let index = self.pop()?;
                    let container = self.pop()?;
                    let v = vm_try!(self.index(container, index));
                    self.push(v)?;
                }
                Op::StoreIndex(slot, depth) => {
                    if slot >= f.num_locals || depth == 0 {
                        return Err(self.error("invalid StoreIndex (invalid bytecode)"));
                    }
                    let value = self.pop()?;
                    let path = self.pop_n(depth as usize)?;
                    let at = base + slot as usize;
                    let mut target = std::mem::replace(&mut self.stack[at], Value::Unit);
                    let result = self.store_path(&mut target, &path, value);
                    self.stack[at] = target;
                    vm_try!(result);
                }
                Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let v = vm_try!(self.arith(op, a, b));
                    self.push(v)?;
                }
                Op::Neg => {
                    let v = match self.pop()? {
                        Value::Int(a) => match a.checked_neg() {
                            Some(v) => Value::Int(v),
                            None => return Err(self.error(format!("integer overflow in `-({a})`"))),
                        },
                        Value::Float(x) => Value::Float(-x),
                        other => {
                            return Err(self.error(format!(
                                "cannot negate {} (invalid bytecode)",
                                other.kind()
                            )))
                        }
                    };
                    self.push(v)?;
                }
                Op::Not => {
                    let b = self.pop_bool()?;
                    self.push(Value::Bool(!b))?;
                }
                Op::Eq | Op::Ne => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let equal = a == b;
                    self.push(Value::Bool(if op == Op::Eq { equal } else { !equal }))?;
                }
                Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    // NaN compares false with everything, as in IEEE 754.
                    let r = match (&a, &b) {
                        (Value::Float(x), Value::Float(y)) => match op {
                            Op::Lt => x < y,
                            Op::Le => x <= y,
                            Op::Gt => x > y,
                            _ => x >= y,
                        },
                        _ => {
                            let ord = vm_try!(compare(&a, &b));
                            match op {
                                Op::Lt => ord.is_lt(),
                                Op::Le => ord.is_le(),
                                Op::Gt => ord.is_gt(),
                                _ => ord.is_ge(),
                            }
                        }
                    };
                    self.push(Value::Bool(r))?;
                }
                Op::Concat => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let (a, b) = (vm_try!(want_str(&a)), vm_try!(want_str(&b)));
                    let mut s = String::with_capacity(a.len() + b.len());
                    s.push_str(&a);
                    s.push_str(&b);
                    let v = vm_try!(self.str_value(s));
                    self.push(v)?;
                }
                Op::Format(n) => {
                    let parts = self.pop_n(n as usize)?;
                    let mut s = String::new();
                    for p in &parts {
                        let _ = write!(s, "{p}");
                        vm_try!(self.check_string(s.len()));
                    }
                    self.push(Value::Str(Rc::from(s)))?;
                }
                Op::Jump(t) => self.frames.last_mut().expect("frame").pc = t as usize,
                Op::JumpIfFalse(t) => {
                    if !self.pop_bool()? {
                        self.frames.last_mut().expect("frame").pc = t as usize;
                    }
                }
                Op::Call(i) => {
                    if m.functions.get(i as usize).map(|c| c.kind) != Some(FnKind::Fn) {
                        return Err(self.error("call to a non-function (invalid bytecode)"));
                    }
                    self.enter(i as usize)?;
                }
                Op::Return => {
                    let v = self.pop()?;
                    let done = self.frames.pop().expect("frame");
                    self.stack.truncate(done.base);
                    if self.frames.is_empty() {
                        return Ok(());
                    }
                    self.push(v)?;
                }
                Op::Say => {
                    let v = self.pop()?;
                    if let Err(e) = writeln!(self.out, "{v}") {
                        return Err(self.error(format!("failed to write output: {e}")));
                    }
                }
                Op::Method(id) => {
                    let Some(mt) = Method::from_id(id).filter(|mt| !mt.mutating()) else {
                        return Err(self.error("invalid method (invalid bytecode)"));
                    };
                    let args = self.pop_n(mt.argc() as usize)?;
                    let recv = self.pop()?;
                    let v = vm_try!(self.method(mt, recv, &args));
                    self.push(v)?;
                }
                Op::MethodMut(slot, id) => {
                    let Some(mt) = Method::from_id(id).filter(|mt| mt.mutating()) else {
                        return Err(self.error("invalid method (invalid bytecode)"));
                    };
                    if slot >= f.num_locals {
                        return Err(self.error("local slot out of range (invalid bytecode)"));
                    }
                    let args = self.pop_n(mt.argc() as usize)?;
                    let at = base + slot as usize;
                    let mut target = std::mem::replace(&mut self.stack[at], Value::Unit);
                    let result = self.method_mut(mt, &mut target, &args);
                    self.stack[at] = target;
                    let v = vm_try!(result);
                    self.push(v)?;
                }
                Op::Builtin(id) => {
                    let Some(b) = Builtin::from_id(id) else {
                        return Err(self.error("invalid built-in (invalid bytecode)"));
                    };
                    let args = self.pop_n(b.argc() as usize)?;
                    let v = vm_try!(self.builtin(b, &args));
                    self.push(v)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_text_forms() {
        let arr = Value::Array(Rc::new(vec![
            Value::Int(1),
            Value::Str(Rc::from("a\"b")),
            Value::Float(2.0),
        ]));
        assert_eq!(arr.to_string(), r#"[1, "a\"b", 2.0]"#);
        let mut m = BTreeMap::new();
        m.insert(Key::Str(Rc::from("b")), Value::Bool(true));
        m.insert(Key::Str(Rc::from("a")), Value::Float(0.5));
        assert_eq!(
            Value::Map(Rc::new(m)).to_string(),
            r#"{"a": 0.5, "b": true}"#
        );
        assert_eq!(Value::Float(0.1 + 0.2).to_string(), "0.30000000000000004");
        assert_eq!(Value::Float(1e20).to_string(), "100000000000000000000");
        assert_eq!(Value::Float(f64::NAN).to_string(), "NaN");
        assert_eq!(Value::Float(-3.0).to_string(), "-3.0");
    }

    #[test]
    fn index_bounds() {
        assert_eq!(index_in(0, 1, false), Ok(0));
        assert!(index_in(1, 1, false).is_err());
        assert!(index_in(-1, 5, false).is_err());
        assert!(index_in(0, 0, false).is_err());
        assert_eq!(index_in(0, 0, true), Ok(0));
        assert_eq!(index_in(3, 3, true), Ok(3));
        assert!(slice_range(2, 1, 5).is_err());
        assert_eq!(slice_range(1, 5, 5), Ok((1, 5)));
    }
}
