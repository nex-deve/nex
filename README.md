# NEX

NEX is a small, statically typed programming language with a Rust compiler,
a bytecode verifier and a stack-based virtual machine.

```nex
app NexDeve {
    let version = 1
    say "Hello, NexDeve!"
    say "NEX version: {version}"
}

fn add(a: Int, b: Int) -> Int {
    return a + b
}

app Calculator {
    say add(10, 20)
}
```

```console
$ nex run examples/hello.nex
Hello, NexDeve!
NEX version: 1
30
```

## Status: alpha (0.1.0-alpha.1)

NEX is **not production-ready**. What works today, with tests:

- `Int`, `Float`, `Bool`, `Str`; `let`/`var`; arithmetic with checked
  integer overflow; comparisons; `&&`/`||` with short-circuiting
- arrays `[T]` and maps `{K: V}` with indexing, element assignment and
  methods (`push`, `pop`, `sorted`, `keys`, `get`, ...); string methods
  (`split`, `trim`, `replace`, `upper`, ...)
- functions, recursion, `if`/`else if`/`else`, `while`, `for` (ranges,
  arrays, map keys), `break`, `continue`
- static type checking with inference, string interpolation
- multiple `app` entry points per file
- compile errors with line, column and source snippet; runtime errors with
  a call trace
- `nex run`, `check`, `build` (→ `.nexdeve` bytecode file), `disasm`, `new`

Not implemented yet: structs, closures, error values, modules/imports
(`use` reports "unresolved module"), file/network I/O, JSON, `nex test`,
`nex fmt`, `nex repl`, package management, the VS Code extension.
See [ROADMAP.md](ROADMAP.md).

**Tested environments:** the full test suite passes in GitHub Actions on
`ubuntu-latest`, `windows-latest` and `macos-latest` (stable Rust), and
locally on Windows 11 x86_64 with rustc 1.99.1 (`x86_64-pc-windows-gnu`).
There are no prebuilt binaries yet; build from source.

## Install from source

Requires a stable Rust toolchain.

```sh
git clone https://github.com/nex-deve/nex.git
cd nex
cargo build --release
# binary: target/release/nex (nex.exe on Windows)
cargo install --path crates/nex-cli    # optional: puts `nex` on PATH
```

On Windows the default MSVC toolchain also needs Visual Studio Build Tools
*with the Windows SDK*. If linking fails, see
[CONTRIBUTING.md](CONTRIBUTING.md#windows-notes).

There are no prebuilt binaries yet.

## Usage

```
nex run [FILE] [--app NAME] [--max-steps N] [--max-call-depth N]
nex check [FILE]
nex build [FILE] [-o OUT.nexdeve]
nex disasm [FILE]
nex new NAME
nex --help | --version
```

| Command | What it does |
|---|---|
| `run` | Compiles a `.nex` file (or loads a `.nexdeve` file), verifies the bytecode, then runs every app in order, or only `--app NAME`. Nothing runs if compilation fails. |
| `check` | Reports syntax, name and type errors without running anything. Prints `ok: ...` on success. |
| `build` | Writes a verified bytecode file (default: same name with `.nexdeve`). Run it with `nex run file.nexdeve`; the source is not needed. A `.nexdeve` file is bytecode for the NEX VM, not a native executable. |
| `disasm` | Prints the bytecode of a `.nex` or `.nexdeve` file. |
| `new` | Creates `NAME/nex.toml`, `NAME/src/main.nex` and `.gitignore`. Inside a project directory, `run`, `check`, `build` and `disasm` use `src/main.nex` when no file is given; `build` writes `target/main.nexdeve`. |

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | compile error (syntax, name, type), or `run` on a file with no app |
| 2 | usage error, unreadable file, or unimplemented command |
| 3 | runtime error (overflow, division by zero, resource limit) |
| 4 | invalid or corrupted `.nexdeve` file |

## Language tour

```nex
// Functions need parameter types; the return type defaults to Unit.
fn label(n: Int) -> Str {
    if n % 15 == 0 {
        return "FizzBuzz"
    } else if n % 3 == 0 {
        return "Fizz"
    }
    return "{n}"                 // interpolation turns any value into text
}

app Demo {
    let limit = 5                // immutable
    var i = 1                    // mutable
    while i <= limit {
        say label(i)
        i = i + 1
    }
    say "done: " + "ok"          // Str + Str concatenation
    say 9223372036854775807 + 1  // runtime error: integer overflow
}
```

The full rules are in [SPEC.md](SPEC.md). More examples are in
[`examples/`](examples); every one is run by the test suite.

## Documentation

- [SPEC.md](SPEC.md) — grammar, types, semantics, limits
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — compiler pipeline,
  bytecode instruction set, verifier, VM, `.nexdeve` format
- [SECURITY.md](SECURITY.md) — what the VM does and does not protect against
- [ROADMAP.md](ROADMAP.md) — milestones and their status
- [CONTRIBUTING.md](CONTRIBUTING.md) — building, testing, adding tests
- [CHANGELOG.md](CHANGELOG.md)

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The tests are unit tests for each compiler stage, golden programs in
`tests/programs/` (expected output or expected error for each `.nex`
file), the examples, end-to-end CLI tests, and deterministic randomized
robustness tests for the parser, verifier, VM and `.nexdeve` decoder.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option. Unless you state otherwise,
any contribution you submit for inclusion in NEX is dual-licensed the same
way, without any additional terms.
