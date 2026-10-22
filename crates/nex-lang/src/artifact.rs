//! `.nexdeve` build artifacts: binary serialization of a bytecode module.
//!
//! Layout (all integers little-endian, strings are `u32` length + UTF-8):
//!
//! ```text
//! magic            4 bytes  "NEXB"
//! format_version   u16      FORMAT_VERSION
//! compiler         str      version of the nex compiler that wrote it
//! source_name      str
//! const_count      u32      then per constant: tag u8
//!                           (0 = Int i64, 1 = Str str, 2 = Float f64 bits)
//! function_count   u32      then per function:
//!     name str, kind u8 (0 = fn, 1 = app), arity u32, num_locals u32,
//!     code_len u32, then per instruction: opcode u8, then 0-2 operands
//!     u32 (count fixed per opcode, see `Op::operand_count`),
//!     then per instruction: line u32, col u32
//! app_count        u32      then per app: function index u32
//! checksum         u64      FNV-1a 64 of every preceding byte
//! ```
//!
//! Decoding checks structure and the checksum only. Callers must run the
//! verifier before executing the module.

use crate::bytecode::{Const, FnKind, Function, Module, Op, Pos};

pub const MAGIC: &[u8; 4] = b"NEXB";
/// Version 2 added floats, collections, methods and built-ins.
pub const FORMAT_VERSION: u16 = 2;

pub fn encode(m: &Module) -> Vec<u8> {
    let mut w = Vec::new();
    w.extend_from_slice(MAGIC);
    w.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    put_str(&mut w, crate::VERSION);
    put_str(&mut w, &m.source_name);
    put_u32(&mut w, m.consts.len() as u32);
    for c in &m.consts {
        match c {
            Const::Int(v) => {
                w.push(0);
                w.extend_from_slice(&v.to_le_bytes());
            }
            Const::Str(s) => {
                w.push(1);
                put_str(&mut w, s);
            }
            Const::Float(v) => {
                w.push(2);
                w.extend_from_slice(&v.to_bits().to_le_bytes());
            }
        }
    }
    put_u32(&mut w, m.functions.len() as u32);
    for f in &m.functions {
        put_str(&mut w, &f.name);
        w.push(match f.kind {
            FnKind::Fn => 0,
            FnKind::App => 1,
        });
        put_u32(&mut w, f.arity);
        put_u32(&mut w, f.num_locals);
        put_u32(&mut w, f.code.len() as u32);
        for op in &f.code {
            w.push(op.opcode());
            for x in op.operands() {
                put_u32(&mut w, x);
            }
        }
        for p in &f.positions {
            put_u32(&mut w, p.line);
            put_u32(&mut w, p.col);
        }
    }
    put_u32(&mut w, m.apps.len() as u32);
    for &a in &m.apps {
        put_u32(&mut w, a);
    }
    let sum = fnv1a64(&w);
    w.extend_from_slice(&sum.to_le_bytes());
    w
}

pub fn decode(bytes: &[u8]) -> Result<Module, String> {
    if bytes.len() < MAGIC.len() || &bytes[..4] != MAGIC {
        return Err("not a NEX bytecode file (bad magic number)".into());
    }
    if bytes.len() < 4 + 2 + 8 {
        return Err("file is truncated".into());
    }
    let (body, sum) = bytes.split_at(bytes.len() - 8);
    let version = u16::from_le_bytes([body[4], body[5]]);
    if version != FORMAT_VERSION {
        return Err(format!(
            "unsupported bytecode format version {version} (this nex reads version {FORMAT_VERSION}); rebuild the program"
        ));
    }
    let expected = u64::from_le_bytes(sum.try_into().expect("8 bytes"));
    if fnv1a64(body) != expected {
        return Err("checksum mismatch: the file is corrupted".into());
    }

    let mut r = Reader { data: body, pos: 6 };
    let _compiler = r.str()?;
    let source_name = r.str()?;
    let n = r.count()?;
    let mut consts = Vec::with_capacity(n);
    for _ in 0..n {
        consts.push(match r.u8()? {
            0 => Const::Int(r.i64()?),
            1 => Const::Str(r.str()?),
            2 => Const::Float(f64::from_bits(r.i64()? as u64)),
            t => return Err(format!("unknown constant tag {t}")),
        });
    }
    let n = r.count()?;
    let mut functions = Vec::with_capacity(n);
    for _ in 0..n {
        let name = r.str()?;
        let kind = match r.u8()? {
            0 => FnKind::Fn,
            1 => FnKind::App,
            k => return Err(format!("unknown function kind {k}")),
        };
        let arity = r.u32()?;
        let num_locals = r.u32()?;
        let len = r.count()?;
        let mut code = Vec::with_capacity(len);
        for _ in 0..len {
            let opcode = r.u8()?;
            let Some(count) = Op::operand_count(opcode) else {
                return Err(format!("unknown opcode 0x{opcode:02x}"));
            };
            let a = if count >= 1 { r.u32()? } else { 0 };
            let b = if count >= 2 { r.u32()? } else { 0 };
            code.push(Op::decode(opcode, a, b).expect("known opcode"));
        }
        let mut positions = Vec::with_capacity(len);
        for _ in 0..len {
            positions.push(Pos {
                line: r.u32()?,
                col: r.u32()?,
            });
        }
        functions.push(Function {
            name,
            kind,
            arity,
            num_locals,
            code,
            positions,
        });
    }
    let n = r.count()?;
    let mut apps = Vec::with_capacity(n);
    for _ in 0..n {
        apps.push(r.u32()?);
    }
    if r.pos != body.len() {
        return Err("unexpected trailing data".into());
    }
    Ok(Module {
        source_name,
        consts,
        functions,
        apps,
    })
}

fn put_u32(w: &mut Vec<u8>, v: u32) {
    w.extend_from_slice(&v.to_le_bytes());
}

fn put_str(w: &mut Vec<u8>, s: &str) {
    put_u32(w, s.len() as u32);
    w.extend_from_slice(s.as_bytes());
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        if self.data.len() - self.pos < n {
            return Err("file is truncated".into());
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }

    /// An element count; rejected if larger than the bytes left, so a
    /// corrupt count cannot trigger a huge allocation.
    fn count(&mut self) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n > self.data.len() - self.pos {
            return Err("element count exceeds file size".into());
        }
        Ok(n)
    }

    fn str(&mut self) -> Result<String, String> {
        let n = self.count()?;
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "invalid UTF-8 in string".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::SourceFile;

    fn sample() -> Module {
        let src = SourceFile::new(
            "t.nex",
            "fn add(a: Int, b: Int) -> Int { return a + b }\napp A { say \"sum {add(1, 2)}\" }",
        );
        crate::compile_source(&src).expect("compiles")
    }

    #[test]
    fn roundtrip() {
        let m = sample();
        let bytes = encode(&m);
        assert_eq!(&bytes[..4], MAGIC);
        assert_eq!(decode(&bytes), Ok(m));
    }

    #[test]
    fn rejects_corruption() {
        let bytes = encode(&sample());
        assert!(decode(b"nope").unwrap_err().contains("magic"));
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        let mut flipped = bytes.clone();
        flipped[20] ^= 0x40;
        assert!(decode(&flipped).unwrap_err().contains("checksum"));
        let mut version = bytes.clone();
        version[4] = 9;
        assert!(decode(&version).unwrap_err().contains("format version"));
    }

    #[test]
    fn every_truncation_is_an_error_not_a_panic() {
        let bytes = encode(&sample());
        for len in 0..bytes.len() {
            assert!(decode(&bytes[..len]).is_err(), "len {len}");
        }
    }
}
