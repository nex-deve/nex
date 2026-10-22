//! Golden tests over `tests/programs/*.nex` and `examples/*.nex`.
//!
//! For each `NAME.nex`:
//! - `NAME.out`: the program must compile and run, printing exactly this.
//! - `NAME.err`: first line is `compile` or `runtime`; every following
//!   non-empty line must appear in the program's stdout plus rendered error.

use nex_lang::diag::Diagnostic;
use nex_lang::span::SourceFile;
use nex_lang::vm::Limits;
use nex_lang::{run_source, RunError};
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn nex_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "nex"))
        .collect();
    files.sort();
    files
}

fn render_compile(diags: &[Diagnostic], src: &SourceFile) -> String {
    diags.iter().map(|d| d.render(src)).collect()
}

/// Returns a failure description, or `None` if the case passed.
fn run_case(path: &Path) -> Option<String> {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let text = fs::read_to_string(path).expect("read source");
    let src = SourceFile::new(name.clone(), text);
    let mut stdout = Vec::new();
    let result = run_source(&src, &mut stdout, &Limits::default());
    let stdout = String::from_utf8(stdout).expect("utf-8 output");

    let out_file = path.with_extension("out");
    let err_file = path.with_extension("err");
    if out_file.exists() {
        let want = fs::read_to_string(&out_file).expect("read .out");
        return match result {
            Ok(()) if stdout == want => None,
            Ok(()) => Some(format!(
                "{name}: stdout mismatch\n--- want\n{want}--- got\n{stdout}"
            )),
            Err(RunError::Compile(d)) => Some(format!(
                "{name}: compile error\n{}",
                render_compile(&d, &src)
            )),
            Err(RunError::Runtime(e)) => {
                Some(format!("{name}: runtime error\n{}", e.render(&name)))
            }
        };
    }
    if err_file.exists() {
        let spec = fs::read_to_string(&err_file).expect("read .err");
        let mut lines = spec.lines();
        let kind = lines.next().unwrap_or_default().trim();
        let rendered = match (&result, kind) {
            (Err(RunError::Compile(d)), "compile") => render_compile(d, &src),
            (Err(RunError::Runtime(e)), "runtime") => format!("{stdout}{}", e.render(&name)),
            _ => {
                return Some(format!(
                    "{name}: expected a {kind} error, got {:?}\nstdout:\n{stdout}",
                    result.err()
                ))
            }
        };
        let missing: Vec<&str> = lines
            .map(str::trim)
            .filter(|l| !l.is_empty() && !rendered.contains(l))
            .collect();
        return if missing.is_empty() {
            None
        } else {
            Some(format!(
                "{name}: output lacks {missing:?}\n--- got\n{rendered}"
            ))
        };
    }
    Some(format!("{name}: no .out or .err file"))
}

fn run_dir(dir: &Path) {
    let files = nex_files(dir);
    assert!(!files.is_empty(), "no .nex files in {}", dir.display());
    let failures: Vec<String> = files.iter().filter_map(|p| run_case(p)).collect();
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n\n{}",
        failures.len(),
        files.len(),
        failures.join("\n\n")
    );
}

#[test]
fn golden_programs() {
    run_dir(&repo_root().join("tests/programs"));
}

#[test]
fn examples() {
    run_dir(&repo_root().join("examples"));
}
