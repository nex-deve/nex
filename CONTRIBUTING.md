# Contributing to NEX

## Setup

You need a stable Rust toolchain (tested with rustc 1.99.1,
`x86_64-pc-windows-gnu`). There are no other dependencies.

```sh
cargo build --workspace
cargo test --workspace
```

### Windows notes

- The default `x86_64-pc-windows-msvc` toolchain needs Visual Studio Build
  Tools **with the Windows SDK**. Without the SDK, linking fails with
  "`link.exe` not found" or missing `kernel32.lib`. Either install the
  "Desktop development with C++" workload, or use the GNU toolchain:
  `rustup toolchain install stable-x86_64-pc-windows-gnu`.
- If `CARGO_TARGET_DIR` is set globally, builds go there instead of
  `./target`; CLI tests work from any target directory.

## Before sending a change

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Where things go

- Language changes: update `SPEC.md` first, then the implementation, then
  tests. Behaviour that is not in the spec should be a compile error.
- Every bug fix needs a regression test, usually a new pair of files in
  `tests/programs/`:
  - `name.nex` + `name.out` — must run and print exactly `name.out`;
  - `name.nex` + `name.err` — first line `compile` or `runtime`, then lines
    that must appear in the error output.
- Examples in `examples/` must have a `.out` file; they are run by the test
  suite.
- Bytecode changes: update `docs/ARCHITECTURE.md`, and bump
  `artifact::FORMAT_VERSION` if the encoding or instruction meanings change.

## Honesty rules for docs and releases

- Do not mark a feature done in `ROADMAP.md` without tests.
- Do not claim a platform is supported unless CI built and tested on it.
- Record known limitations in `CHANGELOG.md` for each release.
