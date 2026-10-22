# Security

## Reporting a vulnerability

Please report security problems privately to the maintainers (NexDeve)
instead of opening a public issue. Include a minimal `.nex` or `.nexdeve` file
that reproduces the problem.

## What NEX does and does not protect against

NEX 0.1.0-alpha.1 is **not a sandbox** and has not been security-audited.

What the current implementation provides:

- **No ambient capabilities in the language.** The language has no file,
  network, process or environment access. The only effect a program can
  have is writing text to standard output. (This will change when the
  standard library gains I/O; a capability model will be documented then.)
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
