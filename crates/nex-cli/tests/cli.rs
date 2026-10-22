//! End-to-end tests of the `nex` binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn nex() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nex"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(args: &[&str]) -> Output {
    nex().args(args).output().expect("spawn nex")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).replace("\r\n", "\n")
}

/// A fresh, empty scratch directory under the cargo target dir.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, text).expect("write file");
    p
}

#[test]
fn version_and_help() {
    let o = run(&["--version"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).starts_with(&format!("nex {}", env!("CARGO_PKG_VERSION"))));
    assert_eq!(run(&["-V"]).status.code(), Some(0));

    let o = run(&["--help"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("USAGE:"));
    assert!(stdout(&o).contains("EXIT CODES:"));

    let o = run(&[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("USAGE:"));
}

#[test]
fn runs_the_specification_sample() {
    let sample = repo_root().join("examples/hello.nex");
    let o = run(&["run", sample.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    assert_eq!(stdout(&o), "Hello, NexDeve!\nNEX version: 1\n30\n");

    let o = run(&["run", sample.to_str().unwrap(), "--app", "Calculator"]);
    assert_eq!(stdout(&o), "30\n");

    let o = run(&["run", sample.to_str().unwrap(), "--app", "Nope"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("available apps: NexDeve, Calculator"));
}

#[test]
fn syntax_and_type_errors_exit_1() {
    let dir = scratch("errors");
    let bad = write(&dir, "bad.nex", "app A {\n    let = 5\n}\n");
    for cmd in ["run", "check", "build"] {
        let o = run(&[cmd, bad.to_str().unwrap()]);
        assert_eq!(o.status.code(), Some(1), "{cmd}");
        let err = stderr(&o);
        assert!(
            err.contains("error: expected variable name"),
            "{cmd}: {err}"
        );
        assert!(err.contains("2 |     let = 5"), "{cmd}: {err}");
        assert!(stdout(&o).is_empty(), "{cmd} must not run anything");
    }
    assert!(
        !dir.join("bad.nexdeve").exists(),
        "build must not write output on error"
    );

    let typed = write(
        &dir,
        "typed.nex",
        "app A {\n    say \"started\"\n    say 1 + true\n}\n",
    );
    let o = run(&["run", typed.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stdout(&o).is_empty(),
        "a program that fails to compile must not start"
    );
    assert!(stderr(&o).contains("cannot apply `+` to `Int` and `Bool`"));

    let web = write(&dir, "web.nex", "use net.http\napp A {}\n");
    let o = run(&["check", web.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("unresolved module `net.http`"));
}

#[test]
fn check_succeeds_without_running() {
    let dir = scratch("check_ok");
    let f = write(&dir, "ok.nex", "app A { say \"should not print\" }\n");
    let o = run(&["check", f.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).starts_with("ok:"));
    assert!(!stdout(&o).contains("should not print"));
}

#[test]
fn runtime_errors_exit_3_with_trace() {
    let dir = scratch("runtime");
    let f = write(
        &dir,
        "div.nex",
        "fn div(a: Int, b: Int) -> Int {\n    return a / b\n}\napp Main {\n    say \"before\"\n    say div(1, 0)\n}\n",
    );
    let o = run(&["run", f.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(3));
    assert_eq!(stdout(&o), "before\n");
    let err = stderr(&o);
    assert!(err.contains("runtime error: division by zero"), "{err}");
    assert!(err.contains("at fn div ("), "{err}");
    assert!(err.contains("at app Main ("), "{err}");
    assert!(err.contains("2 |     return a / b"), "{err}");
}

#[test]
fn execution_limits() {
    let dir = scratch("limits");
    let spin = write(&dir, "spin.nex", "app Spin {\n    while true { }\n}\n");
    let o = run(&["run", spin.to_str().unwrap(), "--max-steps", "1000"]);
    assert_eq!(o.status.code(), Some(3));
    assert!(stderr(&o).contains("step limit of 1000 instructions exceeded"));

    let rec = write(
        &dir,
        "rec.nex",
        "fn f(n: Int) -> Int { return f(n + 1) }\napp R { say f(0) }\n",
    );
    let o = run(&["run", rec.to_str().unwrap(), "--max-call-depth", "50"]);
    assert_eq!(o.status.code(), Some(3));
    assert!(stderr(&o).contains("call depth limit of 50 exceeded"));

    let o = run(&["run", rec.to_str().unwrap(), "--max-steps", "abc"]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn build_writes_a_runnable_artifact() {
    let dir = scratch("build");
    let src = dir.join("hello.nex");
    fs::copy(repo_root().join("examples/hello.nex"), &src).expect("copy sample");

    let o = run(&["build", src.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let artifact = dir.join("hello.nexdeve");
    let bytes = fs::read(&artifact).expect("artifact written");
    assert_eq!(&bytes[..4], b"NEXB");

    // The artifact runs without the source file.
    fs::remove_file(&src).expect("remove source");
    let o = run(&["run", artifact.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(stdout(&o), "Hello, NexDeve!\nNEX version: 1\n30\n");

    let o = run(&["disasm", artifact.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("fn add (index"));

    let custom = dir.join("out/custom.nexdeve");
    fs::write(&src, "app A { say 1 }\n").unwrap();
    let o = run(&[
        "build",
        src.to_str().unwrap(),
        "-o",
        custom.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0));
    assert!(custom.exists());
}

#[test]
fn corrupted_artifacts_exit_4() {
    let dir = scratch("corrupt");
    let src = write(&dir, "a.nex", "app A { say \"hi\" }\n");
    assert_eq!(
        run(&["build", src.to_str().unwrap()]).status.code(),
        Some(0)
    );
    let artifact = dir.join("a.nexdeve");
    let mut bytes = fs::read(&artifact).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    fs::write(&artifact, &bytes).unwrap();
    let o = run(&["run", artifact.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(4));
    assert!(stderr(&o).contains("checksum mismatch"));

    let junk = write(&dir, "junk.nexdeve", "not bytecode");
    let o = run(&["run", junk.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(4));
}

#[test]
fn new_project_runs_and_builds() {
    let dir = scratch("new");
    let o = nex()
        .args(["new", "my-app"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let project = dir.join("my-app");
    assert!(project.join("nex.toml").is_file());
    assert!(project.join("src/main.nex").is_file());

    let o = nex().arg("run").current_dir(&project).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(stdout(&o), "Hello from my-app!\n");

    let o = nex().arg("build").current_dir(&project).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(project.join("target/main.nexdeve").is_file());

    let o = nex()
        .args(["new", "my-app"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("already exists"));

    let o = nex()
        .args(["new", "../escape"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        vec!["frobnicate"],
        vec!["test"],
        vec!["fmt"],
        vec!["repl"],
        vec!["run", "does-not-exist.nex"],
        vec!["check", "--app", "x"],
    ] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", stderr(&o));
    }
    let o = run(&["test"]);
    assert!(stderr(&o).contains("not implemented yet"));
}

#[test]
fn file_without_apps() {
    let dir = scratch("noapp");
    let f = write(&dir, "lib.nex", "fn helper() -> Int { return 1 }\n");
    assert_eq!(run(&["check", f.to_str().unwrap()]).status.code(), Some(0));
    let o = run(&["run", f.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("no `app` block"));
}
