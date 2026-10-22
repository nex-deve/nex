# Roadmap

Status key: **done** = implemented and covered by tests in this repository;
**partial** = some parts done (listed); **planned** = not started.

## Milestone A — core language: **done**

- [x] Int, Bool and Str values; integer, boolean and string literals
- [x] `let` (immutable) and `var` (mutable) bindings, assignment
- [x] Arithmetic (checked overflow) and comparison operators
- [x] `say`
- [x] `app` entry points (multiple per file, `--app` selection)
- [x] Syntax errors with line/column and source snippet

## Milestone B — functions, control flow, data: **done**

- [x] Functions, parameters, return values, recursion
- [x] Lexical scopes and shadowing
- [x] `if` / `else if` / `else`
- [x] `while`, `break`, `continue`
- [x] Static type checking with inference for `let`/`var`
- [x] String interpolation
- [x] `for` loops over ranges, arrays and map keys
- [x] Arrays `[T]` and maps `{K: V}` with value semantics, indexing,
      element assignment and methods
- [x] `Float` (IEEE 754) with explicit `to_float` / `to_int` conversion
- [x] String methods (`split`, `trim`, `replace`, `slice`, ...)
- [x] Built-ins: `sqrt`, `floor`, `round`, `parse_int`, `parse_float`

## Beyond the core (direction, not started)

These are the requested long-term targets. Each needs the milestones
below it first, and none is claimed until it has tests.

| Goal | Needs |
|---|---|
| Automation scripts | Milestone C (files, env, process exit codes) |
| Websites and APIs | C + HTTP server/client + JSON |
| Networking tools | C + TCP/UDP sockets |
| Blockchain experiments | C + hashing (SHA-256), bytes type, big integers |
| Apps | C + a GUI or web front-end story |
| Robotics / hardware | C + serial port / GPIO bindings, timing |
| Speed for large programs | profiling, a faster VM or native compilation |

## Milestone C — modules, I/O, JSON and HTTP: **done**

- [x] Module system: `use`, file-based module resolution, circular imports allowed
- [x] File I/O, user input, args, environment variables, stderr, exit codes
- [x] JSON (`Json` type, `json_parse`, `to_json`)
- [x] HTTP client (via system `curl`) and a single-threaded HTTP server
- [x] Error handling: `fail`, `try` / `catch` across calls
- [x] `--sandbox` mode that disables file and environment access

## Networking: **done**

- [x] TCP client and server, UDP, DNS lookup, read timeouts

## Hashing: **done**

- [x] SHA-256, HMAC-SHA-256, hex, base64 (tested against FIPS 180 / RFC 4231)
- [x] Blockchain example: linked blocks, proof-of-work, validation, HMAC signatures
- [x] Public-key signatures (Ed25519, RFC 8032 vectors) and OS random bytes

## Milestone D — tooling: **partial**

- [x] `nex new` (creates `nex.toml`, `src/main.nex`)
- [x] `nex build` → verified `.nexdeve` bytecode artifact
- [x] `nex disasm`
- [ ] `nex test` test runner
- [ ] `nex fmt` formatter
- [ ] `nex repl`
- [ ] Documentation generator
- [ ] Debugger support
- [ ] Dependency manifest semantics, lockfile, registry (see spec §6 of the
      project brief; nothing implemented)

## Editor support: **planned**

- [ ] VS Code extension: syntax highlighting, brackets, comments
- [ ] Diagnostics via a language server
- [ ] Completion and formatting integration

## Quality and release

- [x] Unit tests for lexer, parser, sema, verifier, bytecode, artifact
- [x] Golden program tests (`tests/programs`, `examples`)
- [x] CLI end-to-end tests
- [x] Deterministic randomized robustness tests
- [ ] Coverage-guided fuzzing (`cargo fuzz`)
- [ ] Benchmarks
- [x] CI (fmt, clippy, tests on Linux, Windows and macOS) — passing on
      GitHub Actions
- [ ] Release workflow with checksums and binaries
- [ ] Verified binaries for each target platform
