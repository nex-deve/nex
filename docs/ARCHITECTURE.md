# Compiler and VM Architecture

```
source ─▶ lexer ─▶ tokens ─▶ parser ─▶ AST ─▶ sema ─▶ compiler ─▶ bytecode ─▶ verifier ─▶ VM
                                              (names +                 │
                                               types)                  └─▶ .nexdeve file (nex build)
```

All stages live in the `nex-lang` crate (`crates/nex-lang/src`). The `nex`
binary (`crates/nex-cli`) only parses arguments, reads files and prints
results. There are no third-party dependencies.

| Stage     | File          | Input → output                     | Errors           |
|-----------|---------------|------------------------------------|------------------|
| Spans     | `span.rs`     | byte offsets → line:column         | —                |
| Lexer     | `lexer.rs`    | `&str` → `Vec<Token>`              | first error      |
| Parser    | `parser.rs`   | tokens → `ast::Program`            | first error      |
| Sema      | `sema.rs`     | AST → `Analysis` (expr types, signatures) | all errors |
| Compiler  | `compiler.rs` | AST + Analysis → `bytecode::Module` | none (assumes sema passed) |
| Verifier  | `verify.rs`   | `Module` → ok / `VerifyError`      | first error      |
| VM        | `vm.rs`       | `Module` + app index → output      | `RuntimeError`   |
| Artifact  | `artifact.rs` | `Module` ⇄ bytes                   | decode errors    |
| Diagnostics | `diag.rs`   | `Diagnostic` → rendered text       | —                |

`nex_lang::compile_source` runs lexer → verifier and returns a verified
module or a list of diagnostics.

## Front end

- **Lexer.** Hand-written. Emits `Newline` tokens as statement terminators
  except inside `()`/`[]`, and collapses runs of blank lines. String
  interpolation is split into literal parts and raw expression source with
  its absolute offset; the parser lexes and parses each expression part
  separately, so diagnostics inside `{...}` point at the right column.
- **Parser.** Recursive descent with precedence climbing for binary
  operators. Each expression gets a unique `ExprId`.
- **Sema.** One pass that does name resolution and type checking together.
  Pass 1 collects every `fn`/`app` signature (so call order does not
  matter); pass 2 checks bodies with a scope stack. Expressions with an
  unknown type get no type, which suppresses cascading errors.
  Output is a map `ExprId → Type` that the compiler uses to choose, for
  example, `Add` versus `Concat`.

## Bytecode

A `Module` has a constant pool (`Int`/`Str`), a function table (both `fn`s
and `app`s, in source order), and the list of app indices.

Each `Function` has `arity`, `num_locals` (parameters occupy the first
`arity` slots) and a `code` vector with one source `Pos` per instruction.

### Instruction set (format 2)

| Opcode | Instruction      | Stack effect           | Notes |
|--------|------------------|------------------------|-------|
| 0x01   | `PushConst i`    | → v                    | `consts[i]` (Int, Float or Str) |
| 0x02   | `PushUnit`       | → ()                   | |
| 0x03   | `PushTrue`       | → true                 | |
| 0x04   | `PushFalse`      | → false                | |
| 0x05   | `Load i`         | → local[i]             | |
| 0x06   | `Store i`        | v →                    | local[i] = v |
| 0x07   | `Pop`            | v →                    | |
| 0x08   | `MakeArray n`    | v1..vn → [v1..vn]      | |
| 0x09   | `MakeMap n`      | k1 v1..kn vn → map     | |
| 0x0a   | `Index`          | c i → c[i]             | bounds / key checked |
| 0x0b   | `StoreIndex s d` | i1..id v →             | local[s][i1]..[id] = v, in place |
| 0x10–0x14 | `Add Sub Mul Div Rem` | a b → r       | Int checked; Float IEEE; `Add` also concatenates arrays |
| 0x15   | `Neg`            | a → -a                 | Int checked; Float |
| 0x16   | `Not`            | b → !b                 | |
| 0x20–0x21 | `Eq Ne`       | a b → Bool             | deep equality |
| 0x22–0x25 | `Lt Le Gt Ge` | a b → Bool             | Int, Float, Str |
| 0x30   | `Concat`         | s1 s2 → s              | Str |
| 0x31   | `Format n`       | v1..vn → s             | text forms joined |
| 0x40   | `Jump t`         |                        | absolute index |
| 0x41   | `JumpIfFalse t`  | b →                    | |
| 0x42   | `Call f`         | a1..ak → r             | k = arity of `f` |
| 0x43   | `Return`         | r →                    | |
| 0x50   | `Say`            | v →                    | prints text form + `\n` |
| 0x60   | `Method m`       | recv a1..ak → r        | non-mutating method; k from the method table |
| 0x61   | `MethodMut s m`  | a1..ak → r             | mutating method applied to local `s` in place |
| 0x62   | `Builtin b`      | a1..ak → r             | built-in function |

Method and built-in ids are defined in `bytecode.rs` (`Method`, `Builtin`).
Opcodes, operand counts and ids are fixed for bytecode format 2. Changing
them requires bumping `artifact::FORMAT_VERSION`.

`for` loops compile to counted loops over two hidden locals (counter and
limit/array), so they need no dedicated instructions.

`&&`/`||` compile to jumps (short-circuit). Unreachable statements after a
`return`, `break` or `continue` are not emitted.

## Verifier

Runs on every module before execution, including modules loaded from
`.nexdeve` files. It checks operand ranges (constants, locals, jump targets,
function indices), that `Call` only targets `fn`s, and runs a stack-depth
data-flow analysis: no underflow, depth ≤ 65,536, equal depth wherever paths
merge, exactly one value at `Return`, and no path falls off the end.

The verifier does **not** check value types. Bytecode produced by the
compiler is type-correct by construction; for hand-crafted bytecode the VM
checks types at run time and reports a runtime error instead of crashing.

## VM

A non-recursive interpreter loop. One shared `Vec<Value>` holds every
frame's locals followed by its operand stack; a separate frame stack holds
`(function, pc, base)`. Because NEX calls do not recurse on the native
stack, deep NEX recursion is bounded only by `Limits::max_call_depth`.

Values: `Int(i64)`, `Float(f64)`, `Bool`, `Str(Rc<str>)`,
`Array(Rc<Vec<Value>>)`, `Map(Rc<BTreeMap<Key, Value>>)`, `Unit`.
Strings are immutable. Arrays and maps have value semantics implemented as
copy-on-write: `Rc::make_mut` copies only when the collection is shared, so
`xs.push(..)` and `xs[i] = ..` on an unshared local are in place.
`StoreIndex` and `MethodMut` temporarily move the local out of its slot
while they update it, so the update never sees a second reference.

## `.nexdeve` artifacts

`nex build` serializes a verified module. The format is documented at the
top of `artifact.rs`: magic `NEXB`, format version, compiler version,
source name, constants, functions (code + positions), app list, and a
trailing FNV-1a-64 checksum. The checksum detects accidental corruption; it
is **not** a cryptographic signature. Decoding bounds every length by the
remaining input size, then the module is verified as usual.

`.nexdeve` files contain bytecode only; they still require the `nex` binary
to run. They are not native executables.
