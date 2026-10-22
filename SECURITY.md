# Security

## Reporting a vulnerability

Please report security problems privately to the maintainers (NexDeve)
instead of opening a public issue. Include a minimal `.nex` or `.nexdeve` file
that reproduces the problem.

## What NEX does and does not protect against

NEX 0.1.0-alpha.1 is **not a sandbox** and has not been security-audited.

What the current implementation provides:

- **File and environment access, with a sandbox switch.** Programs can
  read, write and delete files and read environment variables, like any
  scripting language. `nex run --sandbox` disables all of that; a sandboxed
  program can only use standard input/output, `args`, `exit` and time.
  Programs can also make HTTP requests (through the system `curl`,
  restricted to http/https) and run a local HTTP server bound to
  127.0.0.1, and open raw TCP/UDP sockets (`tcp_listen("0.0.0.0", ...)`.
  listens on every interface, so only do that on purpose). `--sandbox`
  blocks files, environment and network.
- **Bytecode verification.** Every module, including `.nexdeve` files from
  disk, is verified before running (see docs/ARCHITECTURE.md). Malformed
  bytecode is rejected or produces a runtime error; it must never cause
  memory unsafety. The implementation contains no `unsafe` Rust code.
- **Resource limits.** Call depth (default 1000), value-stack size
  (1,000,000 slots), and string length (16 MiB) are always bounded.
  The instruction count is **unlimited by default**; pass
  `--max-steps N` when running code you do not trust, or an infinite loop
  will run forever.
- **Bounded decoding.** The `.nexdeve` decoder never allocates more than the
  input size implies, and every length is bounds-checked.

What it does **not** provide:

- No memory limit beyond the limits above. A program can still use up to
  roughly (stack limit × largest value) memory, e.g. many 16 MiB strings
  held in locals across deep recursion.
- No wall-clock timeout. Use `--max-steps` or an external timeout.
- `.nexdeve` checksums detect corruption, not tampering. They are not signed.
- The parser and verifier are tested with deterministic randomized inputs
  (`crates/nex-lang/tests/fuzz_smoke.rs`), not with coverage-guided fuzzing
  yet.

## Packages

There is no package manager yet. When one is added, installing a package
will never run package code automatically.
