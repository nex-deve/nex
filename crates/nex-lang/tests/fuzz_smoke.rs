//! Deterministic randomized robustness tests ("smoke fuzzing").
//!
//! These are not coverage-guided fuzzers. They feed pseudo-random input to
//! each stage and require that nothing panics and that execution limits
//! hold. Seeds are fixed so failures reproduce exactly.

use nex_lang::artifact;
use nex_lang::bytecode::{Const, FnKind, Function, Module, Op, Pos};
use nex_lang::span::SourceFile;
use nex_lang::verify::verify;
use nex_lang::vm::{run_app, Host, Limits};

/// xorshift64* PRNG.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const FRAGMENTS: &[&str] = &[
    "app",
    "fn",
    "let",
    "var",
    "say",
    "return",
    "if",
    "else",
    "while",
    "break",
    "continue",
    "true",
    "false",
    "use",
    "for",
    "x",
    "y",
    "f",
    "Main",
    "Int",
    "Str",
    "Bool",
    "Unit",
    "0",
    "1",
    "42",
    "9223372036854775807",
    "\"s\"",
    "\"{x}\"",
    "\"\\{\"",
    "\"",
    "in",
    "..",
    "1.5",
    "Float",
    "[Int]",
    "{Str: Int}",
    "xs",
    ".len()",
    ".push(1)",
    ".pop()",
    "[0]",
    "[1, 2]",
    "{\"k\": 1}",
    "sqrt(2.0)",
    "to_int(1e300)",
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    ",",
    ":",
    ";",
    ".",
    "->",
    "+",
    "-",
    "*",
    "/",
    "%",
    "=",
    "==",
    "!=",
    "<",
    "<=",
    ">",
    ">=",
    "&&",
    "||",
    "!",
    "\n",
    " ",
    "//c\n",
    "/*",
    "*/",
    "é",
    "@",
    "\t",
];

fn limits() -> Limits {
    Limits {
        max_steps: Some(20_000),
        max_call_depth: 64,
        max_stack_values: 10_000,
        max_string_bytes: 4096,
        max_collection_len: 1000,
    }
}

/// Run with tight limits and no file or environment access.
fn sandboxed(module: &nex_lang::bytecode::Module, app: u32) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut input: &[u8] = b"line\n";
    let mut host = Host::new(&mut out, &mut err, &mut input);
    host.allow_io = false;
    let _ = run_app(module, app, &mut host, &limits());
}

/// Compile and, if that succeeds, run with tight limits. Must not panic.
fn exercise(text: &str) {
    let src = SourceFile::new("fuzz.nex", text);
    if let Ok(module) = nex_lang::compile_source(&src) {
        verify(&module).expect("compiler output must verify");
        let decoded = artifact::decode(&artifact::encode(&module)).expect("roundtrip");
        assert_eq!(decoded, module);
        for &app in &module.apps {
            sandboxed(&module, app);
        }
    } else {
        // Diagnostics must render without panicking, too.
        for d in nex_lang::compile_source(&src).unwrap_err() {
            let _ = d.render(&src);
        }
    }
}

#[test]
fn random_token_soup() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..3000 {
        let len = 1 + rng.below(40);
        let text: String = (0..len)
            .map(|_| FRAGMENTS[rng.below(FRAGMENTS.len())])
            .collect::<Vec<_>>()
            .join(" ");
        exercise(&text);
    }
}

#[test]
fn mutated_valid_programs() {
    let seeds = [
        include_str!("../../../examples/hello.nex"),
        include_str!("../../../examples/fizzbuzz.nex"),
        include_str!("../../../examples/primes.nex"),
        include_str!("../../../examples/fibonacci.nex"),
        include_str!("../../../examples/collections.nex"),
        include_str!("../../../examples/word_count.nex"),
    ];
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    for seed in seeds {
        let chars: Vec<char> = seed.chars().collect();
        for _ in 0..500 {
            let mut c = chars.clone();
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(c.len());
                match rng.below(3) {
                    0 => {
                        c.remove(at);
                    }
                    1 => {
                        let frag = FRAGMENTS[rng.below(FRAGMENTS.len())];
                        for (i, ch) in frag.chars().enumerate() {
                            c.insert(at + i, ch);
                        }
                    }
                    _ => {
                        let other = rng.below(c.len());
                        c.swap(at, other);
                    }
                }
            }
            exercise(&c.into_iter().collect::<String>());
        }
    }
}

fn random_op(rng: &mut Rng) -> Op {
    let x = rng.below(6) as u32;
    match rng.below(40) {
        35 => Op::TryBegin(x),
        36 => Op::TryEnd,
        37 => Op::Fail,
        27 => Op::MakeArray(x),
        28 => Op::MakeMap(x % 3),
        29 => Op::Index,
        30 => Op::StoreIndex(x, 1 + x % 2),
        31 => Op::Method(rng.below(20) as u32),
        32 => Op::MethodMut(x, 32 + rng.below(6) as u32),
        33 => Op::Builtin(rng.below(8) as u32),
        34 => Op::PushConst(3),
        0 => Op::PushConst(x),
        1 => Op::PushUnit,
        2 => Op::PushTrue,
        3 => Op::PushFalse,
        4 => Op::Load(x),
        5 => Op::Store(x),
        6 => Op::Pop,
        7 => Op::Add,
        8 => Op::Sub,
        9 => Op::Mul,
        10 => Op::Div,
        11 => Op::Rem,
        12 => Op::Neg,
        13 => Op::Not,
        14 => Op::Eq,
        15 => Op::Ne,
        16 => Op::Lt,
        17 => Op::Le,
        18 => Op::Gt,
        19 => Op::Ge,
        20 => Op::Concat,
        21 => Op::Format(x),
        22 => Op::Jump(x),
        23 => Op::JumpIfFalse(x),
        24 => Op::Call(x % 2),
        25 => Op::Return,
        _ => Op::Say,
    }
}

#[test]
fn random_bytecode_is_verified_or_rejected() {
    let mut rng = Rng(0x0123_4567_89ab_cdef);
    let mut accepted = 0;
    for _ in 0..20_000 {
        let mut functions = Vec::new();
        for (i, kind) in [FnKind::App, FnKind::Fn].into_iter().enumerate() {
            let len = 1 + rng.below(8);
            let code: Vec<Op> = (0..len).map(|_| random_op(&mut rng)).collect();
            functions.push(Function {
                name: format!("f{i}"),
                kind,
                arity: if kind == FnKind::App {
                    0
                } else {
                    rng.below(3) as u32
                },
                num_locals: 2 + rng.below(3) as u32,
                positions: vec![Pos { line: 1, col: 1 }; code.len()],
                source: 0,
                code,
            });
        }
        let module = Module {
            sources: vec!["fuzz".into()],
            consts: vec![
                Const::Int(rng.next() as i64),
                Const::Str("s".into()),
                Const::Int(0),
                Const::Float((rng.next() % 1000) as f64 / 7.0),
            ],
            functions,
            apps: vec![0],
        };
        if verify(&module).is_ok() {
            accepted += 1;
            sandboxed(&module, 0);
        }
        let bytes = artifact::encode(&module);
        assert_eq!(artifact::decode(&bytes).as_ref(), Ok(&module));
    }
    assert!(accepted > 0, "generator never produced valid bytecode");
}

#[test]
fn random_bytes_never_decode_into_a_panic() {
    let mut rng = Rng(0x5555_aaaa_1234_9876);
    let valid = artifact::encode(
        &nex_lang::compile_source(&SourceFile::new("a.nex", "app A { say 1 }")).unwrap(),
    );
    for _ in 0..5000 {
        let mut bytes = valid.clone();
        for _ in 0..1 + rng.below(6) {
            let at = rng.below(bytes.len());
            bytes[at] = rng.next() as u8;
        }
        if let Ok(m) = artifact::decode(&bytes) {
            if verify(&m).is_ok() {
                for &app in &m.apps {
                    sandboxed(&m, app);
                }
            }
        }
        let len = rng.below(64);
        let noise: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let _ = artifact::decode(&noise);
    }
}
