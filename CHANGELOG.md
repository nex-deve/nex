# Changelog

All notable changes are recorded here. Versions follow semantic versioning;
before 1.0, minor versions may break compatibility.

## [Unreleased] — 0.1.0-alpha.1

Not tagged or released yet.

### Added

- Stage 2: modules with `use` (`use lib.math` loads `lib/math.nex`);
  `fail` and `try`/`catch` error handling across calls; file, input,
  args, env, stderr, exit and time built-ins; `nex run --sandbox`;
  `nex run file.nex -- args`; string literals inside `{...}`
  interpolation. Bytecode format 3.

- Language (step 1): `Float`; arrays `[T]` and maps `{K: V}` with value
  semantics, literals, indexing and element assignment; `for` loops over
  ranges, arrays and map keys; methods on arrays, strings and maps;
  built-in functions `to_float`, `to_int`, `sqrt`, `floor`, `round`,
  `parse_int`, `parse_float`; string comparison with `<`.
- Bytecode format 2 (collections, methods, built-ins, float constants).
  Format 1 files must be rebuilt.
- Build output extension renamed from `.nexb` to `.nexdeve`.
- Language: `Int`, `Bool`, `Str`; `let`/`var`; assignment; arithmetic with
  checked overflow; comparison and logical operators; `say`; string
  interpolation and escapes; functions with recursion; `if`/`else`,
  `while`, `break`, `continue`; block scopes with shadowing; multiple
  `app` entry points per file.
- Compiler: lexer, parser, combined name resolution and type checking,
  bytecode compiler, bytecode verifier.
- VM: non-recursive stack VM with call-depth, stack-size, string-size and
  optional instruction-count limits.
- `.nexdeve` bytecode file format (format version 1) with checksum.
- CLI: `nex run`, `check`, `build`, `disasm`, `new`, `--help`, `--version`;
  stable exit codes.
- Tests: unit, golden-program, example, CLI end-to-end and randomized
  robustness tests.
- Docs: README, SPEC, architecture, security, roadmap, contributing.
- CI on GitHub Actions: fmt, clippy and tests on Linux, Windows and macOS.

### Known limitations

- No structs/enums, closures or generics.
- No JSON, HTTP or sockets yet.
- `nex test`, `nex fmt` and `nex repl` are not implemented (exit code 2).
- The parser stops at the first syntax error.
- Instruction count is unlimited unless `--max-steps` is given.
- Tested on Linux, Windows and macOS via CI. No prebuilt binaries and no
  VS Code extension yet.
