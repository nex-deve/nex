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

#[test]
fn file_io_automation() {
    let dir = scratch("file_io");
    let script = write(
        &dir,
        "report.nex",
        r#"app Report {
    write_file("data.txt", "apple 3\nmango 12\n")
    append_file("data.txt", "apple 2\n")
    var totals: {Str: Int} = {}
    for line in read_file("data.txt").trim().split("\n") {
        let parts = line.split(" ")
        totals[parts[0]] = totals.get(parts[0], 0) + parse_int(parts[1])
    }
    var report = ""
    for name in totals {
        report = report + "{name}: {totals[name]}\n"
    }
    write_file("report.txt", report)
    say "exists: {file_exists("report.txt")}"
    say list_dir(".")
    delete_file("data.txt")
    say "after delete: {file_exists("data.txt")}"
    try {
        say read_file("missing.txt")
    } catch e {
        say "caught: {e.starts_with("read_file: cannot read")}"
    }
}
"#,
    );
    let o = nex()
        .arg("run")
        .arg(&script)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        "exists: true\n[\"data.txt\", \"report.nex\", \"report.txt\"]\nafter delete: false\ncaught: true\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("report.txt")).unwrap(),
        "apple: 5\nmango: 12\n"
    );
}

#[test]
fn sandbox_blocks_files_and_env() {
    let dir = scratch("sandbox");
    let script = write(
        &dir,
        "s.nex",
        "app S {\n    say \"start\"\n    write_file(\"x.txt\", \"no\")\n}\n",
    );
    let o = nex()
        .args(["run", "--sandbox"])
        .arg(&script)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
    assert_eq!(stdout(&o), "start\n");
    assert!(
        stderr(&o).contains("disabled (--sandbox)"),
        "{}",
        stderr(&o)
    );
    assert!(!dir.join("x.txt").exists());
}

#[test]
fn exit_codes_args_and_stderr() {
    let dir = scratch("exit_args");
    let script = write(
        &dir,
        "e.nex",
        "app E {\n    let a = args()\n    say \"got {a.len()}: {a.join(\"|\")}\"\n    eprint(\"warning!\")\n    if a.contains(\"--fail\") {\n        exit(7)\n    }\n    say \"normal end\"\n}\n",
    );
    let o = run(&["run", script.to_str().unwrap(), "--", "one", "--fail"]);
    assert_eq!(o.status.code(), Some(7), "{}", stderr(&o));
    assert_eq!(stdout(&o), "got 2: one|--fail\n");
    assert_eq!(stderr(&o), "warning!\n");

    let o = run(&["run", script.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(stdout(&o), "got 0: \nnormal end\n");
}

#[test]
fn input_from_stdin() {
    use std::io::Write as _;
    let dir = scratch("stdin");
    let script = write(
        &dir,
        "ask.nex",
        "app Ask {\n    let n = parse_int(input(\"number? \"))\n    say \"double: {n * 2}\"\n}\n",
    );
    let mut child = nex()
        .arg("run")
        .arg(&script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"21\n").unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(stdout(&o), "number? double: 42\n");
}

#[test]
fn modules_from_the_cli() {
    let project = repo_root().join("tests/projects/modules");
    let o = run(&["run", project.join("main.nex").to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(stdout(&o).starts_with("49\n25\nHELLO!\n"));

    // a built file contains the modules and runs without them
    let dir = scratch("modules_build");
    let out = dir.join("app.nexdeve");
    let o = run(&[
        "build",
        project.join("main.nex").to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let o = run(&["run", out.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(stdout(&o).contains("==== NEX ===="));

    // errors inside a module are shown against the module file
    let o = run(&[
        "check",
        repo_root()
            .join("tests/projects/module_type_errors/main.nex")
            .to_str()
            .unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(1));
    let err = stderr(&o);
    assert!(err.contains("--> helpers.nex:2:18"), "{err}");
    assert!(err.contains("2 |     return n * 2 + \"a\""), "{err}");
}

#[test]
fn http_server_and_client() {
    use std::io::{BufRead, BufReader, Read, Write as _};
    let dir = scratch("http");
    let server = write(
        &dir,
        "server.nex",
        r#"app Server {
    let s = http_listen(0)
    say "port {http_port(s)}"
    for i in 0..3 {
        let req = http_accept(s)
        if req["path"] == "/hello" {
            let reply = {"hello": json(req["query"]), "method": json(req["method"])}
            http_respond(req["id"], 200, "application/json", to_json(reply))
        } else if req["method"] == "POST" {
            http_respond(req["id"], 201, "text/plain", req["body"].upper())
        } else {
            http_respond(req["id"], 404, "text/plain", "nothing at {req["path"]}")
        }
    }
}
"#,
    );
    let mut child = nex()
        .arg("run")
        .arg(&server)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    out.read_line(&mut first).unwrap();
    let port: u16 = first.trim().strip_prefix("port ").unwrap().parse().unwrap();
    let base = format!("http://127.0.0.1:{port}");

    // 1. A raw HTTP request, no curl involved.
    let mut tcp = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.write_all(b"GET /hello?name=nex HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let mut raw = String::new();
    tcp.read_to_string(&mut raw).unwrap();
    assert!(raw.starts_with("HTTP/1.1 200 OK\r\n"), "{raw}");
    assert!(raw.contains("Content-Type: application/json"), "{raw}");
    assert!(
        raw.ends_with(r#"{"hello":"name=nex","method":"GET"}"#),
        "{raw}"
    );

    // 2. The NEX client (uses curl) for POST and a 404 error.
    let client = write(
        &dir,
        "client.nex",
        r#"app Client {
    let base = args()[0]
    say http_post("{base}/echo", "make me loud", "text/plain")
    try {
        say http_get("{base}/missing")
    } catch e {
        say "caught: {e.contains("returned HTTP 404")}"
    }
    try {
        say http_get("ftp://example.com")
    } catch e {
        say e
    }
}
"#,
    );
    let o = run(&["run", client.to_str().unwrap(), "--", &base]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        "MAKE ME LOUD\ncaught: true\nhttp_get: the URL must start with http:// or https://\n"
    );

    let status = child.wait().unwrap();
    assert!(status.success());

    // 3. The sandbox blocks the network.
    let o = run(&["run", "--sandbox", client.to_str().unwrap(), "--", &base]);
    assert_eq!(o.status.code(), Some(3));
    assert!(stderr(&o).contains("network access is disabled (--sandbox)"));
}
