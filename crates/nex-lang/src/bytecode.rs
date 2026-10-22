//! Bytecode instruction set and module layout. See docs/ARCHITECTURE.md.

use std::fmt::Write;

/// One VM instruction. Every operand is a `u32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Push `consts[i]`.
    PushConst(u32),
    PushUnit,
    PushTrue,
    PushFalse,
    /// Push local slot `i` of the current frame.
    Load(u32),
    /// Pop into local slot `i`.
    Store(u32),
    Pop,
    /// Pop `n` values, push an array of them (first pushed = index 0).
    MakeArray(u32),
    /// Pop `n` key/value pairs (k1 v1 k2 v2 ...), push a map.
    MakeMap(u32),
    /// Pop index/key and container, push the element.
    Index,
    /// `StoreIndex(slot, depth)`: the stack holds `i1 .. i_depth value`
    /// (value on top). Pops them and stores the value at
    /// `local[slot][i1]..[i_depth]`, in place.
    StoreIndex(u32, u32),
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Neg,
    Not,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// Pop two strings, push their concatenation.
    Concat,
    /// Pop `n` values, push the concatenation of their text forms.
    Format(u32),
    /// Jump to absolute instruction index.
    Jump(u32),
    /// Pop a Bool; jump if it is false.
    JumpIfFalse(u32),
    /// Call `functions[i]`; its arguments are the top `arity` values.
    Call(u32),
    /// Pop the return value, discard the frame, push the value to the caller.
    Return,
    /// Install an error handler whose `catch` code starts at `t`. On an
    /// error, the frame's operand stack is reset to its height here, the
    /// message (Str) is pushed and execution continues at `t`.
    TryBegin(u32),
    /// Remove the innermost handler of the current frame.
    TryEnd,
    /// Pop a Str and raise it as an error.
    Fail,
    /// Pop a value and print it followed by a newline.
    Say,
    /// Pop the arguments and the receiver of non-mutating method `id`, push
    /// the result.
    Method(u32),
    /// `MethodMut(slot, id)`: pop the arguments of mutating method `id`,
    /// apply it to local `slot` in place, push the result.
    MethodMut(u32, u32),
    /// Pop the arguments of built-in function `id`, push the result.
    Builtin(u32),
}

impl Op {
    pub fn opcode(self) -> u8 {
        match self {
            Op::PushConst(_) => 0x01,
            Op::PushUnit => 0x02,
            Op::PushTrue => 0x03,
            Op::PushFalse => 0x04,
            Op::Load(_) => 0x05,
            Op::Store(_) => 0x06,
            Op::Pop => 0x07,
            Op::MakeArray(_) => 0x08,
            Op::MakeMap(_) => 0x09,
            Op::Index => 0x0a,
            Op::StoreIndex(..) => 0x0b,
            Op::Add => 0x10,
            Op::Sub => 0x11,
            Op::Mul => 0x12,
            Op::Div => 0x13,
            Op::Rem => 0x14,
            Op::Neg => 0x15,
            Op::Not => 0x16,
            Op::Eq => 0x20,
            Op::Ne => 0x21,
            Op::Lt => 0x22,
            Op::Le => 0x23,
            Op::Gt => 0x24,
            Op::Ge => 0x25,
            Op::Concat => 0x30,
            Op::Format(_) => 0x31,
            Op::Jump(_) => 0x40,
            Op::JumpIfFalse(_) => 0x41,
            Op::Call(_) => 0x42,
            Op::Return => 0x43,
            Op::TryBegin(_) => 0x44,
            Op::TryEnd => 0x45,
            Op::Fail => 0x46,
            Op::Say => 0x50,
            Op::Method(_) => 0x60,
            Op::MethodMut(..) => 0x61,
            Op::Builtin(_) => 0x62,
        }
    }

    /// The operands of this instruction, in encoding order.
    pub fn operands(self) -> Vec<u32> {
        match self {
            Op::PushConst(x)
            | Op::Load(x)
            | Op::Store(x)
            | Op::MakeArray(x)
            | Op::MakeMap(x)
            | Op::Format(x)
            | Op::Jump(x)
            | Op::JumpIfFalse(x)
            | Op::Call(x)
            | Op::TryBegin(x)
            | Op::Method(x)
            | Op::Builtin(x) => vec![x],
            Op::StoreIndex(a, b) | Op::MethodMut(a, b) => vec![a, b],
            _ => Vec::new(),
        }
    }

    /// Number of `u32` operands an opcode byte takes; `None` if unknown.
    pub fn operand_count(opcode: u8) -> Option<usize> {
        match opcode {
            0x01 | 0x05 | 0x06 | 0x08 | 0x09 | 0x31 | 0x40 | 0x41 | 0x42 | 0x44 | 0x60 | 0x62 => {
                Some(1)
            }
            0x0b | 0x61 => Some(2),
            0x02..=0x04
            | 0x07
            | 0x0a
            | 0x10..=0x16
            | 0x20..=0x25
            | 0x30
            | 0x43
            | 0x45
            | 0x46
            | 0x50 => Some(0),
            _ => None,
        }
    }

    /// Rebuild an instruction from its opcode and operands.
    pub fn decode(opcode: u8, a: u32, b: u32) -> Option<Op> {
        Some(match opcode {
            0x01 => Op::PushConst(a),
            0x02 => Op::PushUnit,
            0x03 => Op::PushTrue,
            0x04 => Op::PushFalse,
            0x05 => Op::Load(a),
            0x06 => Op::Store(a),
            0x07 => Op::Pop,
            0x08 => Op::MakeArray(a),
            0x09 => Op::MakeMap(a),
            0x0a => Op::Index,
            0x0b => Op::StoreIndex(a, b),
            0x10 => Op::Add,
            0x11 => Op::Sub,
            0x12 => Op::Mul,
            0x13 => Op::Div,
            0x14 => Op::Rem,
            0x15 => Op::Neg,
            0x16 => Op::Not,
            0x20 => Op::Eq,
            0x21 => Op::Ne,
            0x22 => Op::Lt,
            0x23 => Op::Le,
            0x24 => Op::Gt,
            0x25 => Op::Ge,
            0x30 => Op::Concat,
            0x31 => Op::Format(a),
            0x40 => Op::Jump(a),
            0x41 => Op::JumpIfFalse(a),
            0x42 => Op::Call(a),
            0x43 => Op::Return,
            0x44 => Op::TryBegin(a),
            0x45 => Op::TryEnd,
            0x46 => Op::Fail,
            0x50 => Op::Say,
            0x60 => Op::Method(a),
            0x61 => Op::MethodMut(a, b),
            0x62 => Op::Builtin(a),
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Op::PushConst(_) => "PushConst",
            Op::PushUnit => "PushUnit",
            Op::PushTrue => "PushTrue",
            Op::PushFalse => "PushFalse",
            Op::Load(_) => "Load",
            Op::Store(_) => "Store",
            Op::Pop => "Pop",
            Op::MakeArray(_) => "MakeArray",
            Op::MakeMap(_) => "MakeMap",
            Op::Index => "Index",
            Op::StoreIndex(..) => "StoreIndex",
            Op::Add => "Add",
            Op::Sub => "Sub",
            Op::Mul => "Mul",
            Op::Div => "Div",
            Op::Rem => "Rem",
            Op::Neg => "Neg",
            Op::Not => "Not",
            Op::Eq => "Eq",
            Op::Ne => "Ne",
            Op::Lt => "Lt",
            Op::Le => "Le",
            Op::Gt => "Gt",
            Op::Ge => "Ge",
            Op::Concat => "Concat",
            Op::Format(_) => "Format",
            Op::Jump(_) => "Jump",
            Op::JumpIfFalse(_) => "JumpIfFalse",
            Op::Call(_) => "Call",
            Op::Return => "Return",
            Op::TryBegin(_) => "TryBegin",
            Op::TryEnd => "TryEnd",
            Op::Fail => "Fail",
            Op::Say => "Say",
            Op::Method(_) => "Method",
            Op::MethodMut(..) => "MethodMut",
            Op::Builtin(_) => "Builtin",
        }
    }
}

macro_rules! id_enum {
    ($(#[$doc:meta])* $name:ident { $($variant:ident = $id:literal, $text:literal, $argc:literal, $mutating:literal;)* }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),* }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),*];

            pub fn id(self) -> u32 {
                match self { $($name::$variant => $id),* }
            }

            pub fn from_id(id: u32) -> Option<$name> {
                match id { $($id => Some($name::$variant),)* _ => None }
            }

            pub fn name(self) -> &'static str {
                match self { $($name::$variant => $text),* }
            }

            /// Arguments taken, not counting a method's receiver.
            pub fn argc(self) -> u32 {
                match self { $($name::$variant => $argc),* }
            }

            pub fn mutating(self) -> bool {
                match self { $($name::$variant => $mutating),* }
            }
        }
    };
}

id_enum! {
    /// Methods on arrays, strings and maps. Types are checked by `sema`.
    Method {
        Len = 0, "len", 0, false;
        Contains = 1, "contains", 1, false;
        IndexOf = 2, "index_of", 1, false;
        Join = 3, "join", 1, false;
        Sorted = 4, "sorted", 0, false;
        Reversed = 5, "reversed", 0, false;
        Slice = 6, "slice", 2, false;
        Upper = 7, "upper", 0, false;
        Lower = 8, "lower", 0, false;
        Trim = 9, "trim", 0, false;
        StartsWith = 10, "starts_with", 1, false;
        EndsWith = 11, "ends_with", 1, false;
        Split = 12, "split", 1, false;
        Chars = 13, "chars", 0, false;
        Replace = 14, "replace", 2, false;
        Has = 15, "has", 1, false;
        Get = 16, "get", 2, false;
        Keys = 17, "keys", 0, false;
        Values = 18, "values", 0, false;
        Push = 32, "push", 1, true;
        Pop = 33, "pop", 0, true;
        Insert = 34, "insert", 2, true;
        RemoveAt = 35, "remove_at", 1, true;
        Remove = 36, "remove", 1, true;
        Clear = 37, "clear", 0, true;
        JsonGet = 40, "get", 1, false;
        JsonAt = 41, "at", 1, false;
        JsonHas = 42, "has", 1, false;
        JsonKeys = 43, "keys", 0, false;
        JsonStr = 44, "str", 0, false;
        JsonInt = 45, "int", 0, false;
        JsonFloat = 46, "float", 0, false;
        JsonBool = 47, "bool", 0, false;
        JsonIsNull = 48, "is_null", 0, false;
        JsonKind = 49, "kind", 0, false;
        JsonItems = 50, "items", 0, false;
    }
}

id_enum! {
    /// Built-in functions callable by name.
    Builtin {
        ToFloat = 0, "to_float", 1, false;
        ToInt = 1, "to_int", 1, false;
        Sqrt = 2, "sqrt", 1, false;
        Floor = 3, "floor", 1, false;
        Round = 4, "round", 1, false;
        ParseInt = 5, "parse_int", 1, false;
        ParseFloat = 6, "parse_float", 1, false;
        Input = 7, "input", 1, false;
        ReadFile = 8, "read_file", 1, false;
        WriteFile = 9, "write_file", 2, false;
        AppendFile = 10, "append_file", 2, false;
        FileExists = 11, "file_exists", 1, false;
        ListDir = 12, "list_dir", 1, false;
        DeleteFile = 13, "delete_file", 1, false;
        Args = 14, "args", 0, false;
        Env = 15, "env", 2, false;
        Exit = 16, "exit", 1, false;
        Eprint = 17, "eprint", 1, false;
        TimeMs = 18, "time_ms", 0, false;
        SleepMs = 19, "sleep_ms", 1, false;
        JsonParse = 20, "json_parse", 1, false;
        ToJson = 21, "to_json", 1, false;
        Json = 22, "json", 1, false;
        HttpGet = 23, "http_get", 1, false;
        HttpPost = 24, "http_post", 3, false;
        HttpListen = 25, "http_listen", 1, false;
        HttpPort = 26, "http_port", 1, false;
        HttpAccept = 27, "http_accept", 1, false;
        HttpRespond = 28, "http_respond", 4, false;
        TcpConnect = 29, "tcp_connect", 2, false;
        TcpListen = 30, "tcp_listen", 2, false;
        TcpAccept = 31, "tcp_accept", 1, false;
        TcpSend = 32, "tcp_send", 2, false;
        TcpRecv = 33, "tcp_recv", 2, false;
        TcpRecvLine = 34, "tcp_recv_line", 1, false;
        NetClose = 35, "net_close", 1, false;
        UdpBind = 36, "udp_bind", 2, false;
        UdpSend = 37, "udp_send", 4, false;
        UdpRecv = 38, "udp_recv", 2, false;
        LocalPort = 39, "local_port", 1, false;
        NetTimeout = 40, "net_timeout", 2, false;
        Resolve = 41, "resolve", 1, false;
        Sha256 = 42, "sha256", 1, false;
        HmacSha256 = 43, "hmac_sha256", 2, false;
        HexEncode = 44, "hex_encode", 1, false;
        Base64Encode = 45, "base64_encode", 1, false;
        Base64Decode = 46, "base64_decode", 1, false;
        RandomHex = 47, "random_hex", 1, false;
        Ed25519Keypair = 48, "ed25519_keypair", 0, false;
        Ed25519Public = 49, "ed25519_public", 1, false;
        Ed25519Sign = 50, "ed25519_sign", 2, false;
        Ed25519Verify = 51, "ed25519_verify", 3, false;
        Sha512 = 52, "sha512", 1, false;
    }
}

impl Builtin {
    pub fn by_name(name: &str) -> Option<Builtin> {
        Builtin::ALL.iter().copied().find(|b| b.name() == name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    Int(i64),
    Float(f64),
    Str(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FnKind {
    Fn,
    App,
}

/// 1-based source position of an instruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub kind: FnKind,
    pub arity: u32,
    /// Total local slots, including the `arity` parameter slots.
    pub num_locals: u32,
    pub code: Vec<Op>,
    /// Source position of each instruction; same length as `code`.
    pub positions: Vec<Pos>,
    /// Index into [`Module::sources`] of the file that declared it.
    pub source: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    /// Names of the source files; `sources[0]` is the main file.
    pub sources: Vec<String>,
    pub consts: Vec<Const>,
    pub functions: Vec<Function>,
    /// Indices into `functions` of the apps, in declaration order.
    pub apps: Vec<u32>,
}

impl Module {
    /// Name of the source file that declared `f`.
    pub fn source_of(&self, f: &Function) -> &str {
        self.sources
            .get(f.source as usize)
            .map_or("<unknown>", String::as_str)
    }

    pub fn app_names(&self) -> Vec<&str> {
        self.apps
            .iter()
            .filter_map(|&i| self.functions.get(i as usize))
            .map(|f| f.name.as_str())
            .collect()
    }

    pub fn find_app(&self, name: &str) -> Option<u32> {
        self.apps.iter().copied().find(|&i| {
            self.functions
                .get(i as usize)
                .is_some_and(|f| f.name == name)
        })
    }
}

/// Human-readable listing of a module, used by `nex disasm`.
pub fn disassemble(m: &Module) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "; module {}", m.sources.join(", "));
    let _ = writeln!(out, "; constants");
    for (i, c) in m.consts.iter().enumerate() {
        let _ = match c {
            Const::Int(v) => writeln!(out, "  #{i:<4} Int {v}"),
            Const::Float(v) => writeln!(out, "  #{i:<4} Float {v:?}"),
            Const::Str(s) => writeln!(out, "  #{i:<4} Str {s:?}"),
        };
    }
    for (i, f) in m.functions.iter().enumerate() {
        let kind = match f.kind {
            FnKind::Fn => "fn",
            FnKind::App => "app",
        };
        let _ = writeln!(
            out,
            "\n{kind} {} (index {i}, arity {}, locals {})",
            f.name, f.arity, f.num_locals
        );
        for (pc, op) in f.code.iter().enumerate() {
            let pos = f.positions.get(pc).copied().unwrap_or_default();
            let loc = format!("{}:{}", pos.line, pos.col);
            let _ = write!(out, "  {pc:04}  {loc:<8} {}", op.name());
            for x in op.operands() {
                let _ = write!(out, " {x}");
            }
            let comment = match *op {
                Op::PushConst(x) => match m.consts.get(x as usize) {
                    Some(Const::Int(v)) => Some(v.to_string()),
                    Some(Const::Float(v)) => Some(format!("{v:?}")),
                    Some(Const::Str(s)) => Some(format!("{s:?}")),
                    None => None,
                },
                Op::Call(x) => m.functions.get(x as usize).map(|c| c.name.clone()),
                Op::Method(x) | Op::MethodMut(_, x) => {
                    Method::from_id(x).map(|mt| mt.name().to_string())
                }
                Op::Builtin(x) => Builtin::from_id(x).map(|b| b.name().to_string()),
                _ => None,
            };
            if let Some(c) = comment {
                let _ = write!(out, "  ; {c}");
            }
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_roundtrip() {
        let ops = [
            Op::PushConst(7),
            Op::PushUnit,
            Op::PushTrue,
            Op::PushFalse,
            Op::Load(1),
            Op::Store(2),
            Op::Pop,
            Op::MakeArray(3),
            Op::MakeMap(2),
            Op::Index,
            Op::StoreIndex(1, 2),
            Op::Add,
            Op::Sub,
            Op::Mul,
            Op::Div,
            Op::Rem,
            Op::Neg,
            Op::Not,
            Op::Eq,
            Op::Ne,
            Op::Lt,
            Op::Le,
            Op::Gt,
            Op::Ge,
            Op::Concat,
            Op::Format(3),
            Op::Jump(4),
            Op::JumpIfFalse(5),
            Op::Call(6),
            Op::Return,
            Op::TryBegin(9),
            Op::TryEnd,
            Op::Fail,
            Op::Say,
            Op::Method(4),
            Op::MethodMut(1, 32),
            Op::Builtin(2),
        ];
        for op in ops {
            let code = op.opcode();
            let operands = op.operands();
            assert_eq!(Op::operand_count(code), Some(operands.len()), "{op:?}");
            let a = operands.first().copied().unwrap_or(0);
            let b = operands.get(1).copied().unwrap_or(0);
            assert_eq!(Op::decode(code, a, b), Some(op));
        }
        assert_eq!(Op::operand_count(0xff), None);
        assert_eq!(Op::decode(0xff, 0, 0), None);
    }

    #[test]
    fn method_and_builtin_ids_are_unique() {
        for m in Method::ALL {
            assert_eq!(Method::from_id(m.id()), Some(*m));
        }
        for b in Builtin::ALL {
            assert_eq!(Builtin::from_id(b.id()), Some(*b));
            assert_eq!(Builtin::by_name(b.name()), Some(*b));
        }
    }
}
