//! Golden tests over `tests/programs/*.nex`, `examples/*.nex` and the
//! multi-file projects in `tests/projects/*/`.
//!
//! For each program `NAME.nex` (or project directory with `main.nex`):
//! - `NAME.out` / `expected.out`: it must compile and run, printing exactly this.
//! - `NAME.err` / `expected.err`: first line is `compile` or `runtime`; every
//!   following non-empty line must appear in stdout plus the rendered error.
//! - optional `NAME.in` / `input.txt`: fed to standard input.

use nex_lang::project::FsLoader;
use nex_lang::span::SourceFile;
use nex_lang::vm::{run_app, Host, Limits};
use nex_lang::{compile_project, CompileError};
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

enum Outcome {
    Ok(String),
    Compile(String),
    Runtime(String),
}

/// Compile with modules resolved next to `main`, then run every app.
fn run(main: &Path, name: &str, stdin: &str) -> Outcome {
    let text = fs::read_to_string(main).expect("read source");
    let src = SourceFile::new(name, text);
    let root = main.parent().expect("parent dir");
    let module = match compile_project(src, root, &mut FsLoader) {
        Ok((module, _)) => module,
        Err(e @ CompileError { .. }) => return Outcome::Compile(e.render()),
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut input = stdin.as_bytes();
    let mut host = Host::new(&mut out, &mut err, &mut input);
    host.args = vec!["first".into(), "second".into()];
    let mut failure = None;
    for &app in &module.apps {
        match run_app(&module, app, &mut host, &Limits::default()) {
            Ok(None) => {}
            Ok(Some(code)) => {
                failure = None;
                let _ = code;
                break;
            }
            Err(e) => {
                failure = Some(e.render());
                break;
            }
        }
    }
    let mut text = String::from_utf8(out).expect("utf-8 output");
    text.push_str(&String::from_utf8(err).expect("utf-8 stderr"));
    match failure {
        None => Outcome::Ok(text),
        Some(rendered) => Outcome::Runtime(format!("{text}{rendered}")),
    }
}

/// Returns a failure description, or `None` if the case passed.
fn check(name: &str, outcome: Outcome, out_file: &Path, err_file: &Path) -> Option<String> {
    if out_file.exists() {
        let want = fs::read_to_string(out_file).expect("read .out");
        return match outcome {
            Outcome::Ok(got) if got == want => None,
            Outcome::Ok(got) => Some(format!(
                "{name}: output mismatch\n--- want\n{want}--- got\n{got}"
            )),
            Outcome::Compile(r) => Some(format!("{name}: compile error\n{r}")),
            Outcome::Runtime(r) => Some(format!("{name}: runtime error\n{r}")),
        };
    }
    if err_file.exists() {
        let spec = fs::read_to_string(err_file).expect("read .err");
        let mut lines = spec.lines();
        let kind = lines.next().unwrap_or_default().trim();
        let rendered = match (outcome, kind) {
            (Outcome::Compile(r), "compile") => r,
            (Outcome::Runtime(r), "runtime") => r,
            (Outcome::Ok(out), _) => {
                return Some(format!(
                    "{name}: expected a {kind} error, but it ran:\n{out}"
                ))
            }
            (Outcome::Compile(r) | Outcome::Runtime(r), _) => {
                return Some(format!("{name}: expected a {kind} error, got:\n{r}"))
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
    Some(format!("{name}: no expected output file"))
}

fn report(failures: Vec<String>, total: usize) {
    assert!(total > 0, "no test cases found");
    assert!(
        failures.is_empty(),
        "{} of {total} cases failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

fn run_dir(dir: &Path) {
    let files = nex_files(dir);
    let failures: Vec<String> = files
        .iter()
        .filter_map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let stdin = fs::read_to_string(path.with_extension("in")).unwrap_or_default();
            let outcome = run(path, &name, &stdin);
            check(
                &name,
                outcome,
                &path.with_extension("out"),
                &path.with_extension("err"),
            )
        })
        .collect();
    report(failures, files.len());
}

#[test]
fn golden_programs() {
    run_dir(&repo_root().join("tests/programs"));
}

#[test]
fn examples() {
    run_dir(&repo_root().join("examples"));
}

#[test]
fn projects() {
    let dir = repo_root().join("tests/projects");
    let mut projects: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("tests/projects")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.join("main.nex").is_file())
        .collect();
    projects.sort();
    let failures: Vec<String> = projects
        .iter()
        .filter_map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let stdin = fs::read_to_string(p.join("input.txt")).unwrap_or_default();
            let outcome = run(&p.join("main.nex"), "main.nex", &stdin);
            check(
                &name,
                outcome,
                &p.join("expected.out"),
                &p.join("expected.err"),
            )
        })
        .collect();
    report(failures, projects.len());
}
