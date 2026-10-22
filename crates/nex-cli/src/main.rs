//! The `nex` command-line tool.

use nex_lang::artifact;
use nex_lang::bytecode::{disassemble, Module};
use nex_lang::diag::render_snippet;
use nex_lang::span::SourceFile;
use nex_lang::verify::verify;
use nex_lang::vm::{self, Limits, RuntimeError};
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Exit codes. Documented in README.md; keep them stable.
const EXIT_OK: u8 = 0;
const EXIT_COMPILE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_RUNTIME: u8 = 3;
const EXIT_BAD_ARTIFACT: u8 = 4;

const HELP: &str = "\
nex - the NEX programming language

USAGE:
    nex <COMMAND> [OPTIONS]

COMMANDS:
    run [FILE]        Compile and run a .nex file, or run a built .nexdeve file
    check [FILE]      Check a .nex file for syntax and type errors without running it
    build [FILE]      Compile a .nex file to a verified bytecode file (.nexdeve)
    disasm [FILE]     Print the bytecode of a .nex or .nexdeve file
    new <NAME>        Create a new project directory
    help              Show this help
    version           Show the version

    When FILE is omitted and ./nex.toml exists, src/main.nex is used.

OPTIONS:
    --app <NAME>            (run) Run only the app named NAME
    --max-steps <N>         (run) Stop after N VM instructions
    --max-call-depth <N>    (run) Maximum call depth (default 1000)
    -o, --output <PATH>     (build) Output path (default: FILE with .nexdeve extension,
                            or target/main.nexdeve inside a project)
    -h, --help              Show this help
    -V, --version           Show the version

EXIT CODES:
    0  success
    1  compile error (syntax, name or type error)
    2  usage error, I/O error or unimplemented command
    3  runtime error
    4  invalid or corrupted .nexdeve file

Not implemented yet: nex test, nex fmt, nex repl (see ROADMAP.md).
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(run_cli(&args))
}

fn run_cli(args: &[String]) -> u8 {
    let Some(cmd) = args.first() else {
        eprint!("{HELP}");
        return EXIT_USAGE;
    };
    let rest = &args[1..];
    let result = match cmd.as_str() {
        "-h" | "--help" | "help" => {
            print!("{HELP}");
            Ok(EXIT_OK)
        }
        "-V" | "--version" | "version" => {
            println!(
                "nex {} (bytecode format {})",
                nex_lang::VERSION,
                artifact::FORMAT_VERSION
            );
            Ok(EXIT_OK)
        }
        "run" => cmd_run(rest),
        "check" => cmd_check(rest),
        "build" => cmd_build(rest),
        "disasm" => cmd_disasm(rest),
        "new" => cmd_new(rest),
        "test" | "fmt" | "repl" => Err(format!(
            "`nex {cmd}` is not implemented yet (planned for milestone D; see ROADMAP.md)"
        )),
        other => Err(format!(
            "unknown command `{other}`\n\nRun `nex --help` for usage."
        )),
    };
    match result {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            EXIT_USAGE
        }
    }
}

/// `Ok(code)`: finished (errors already printed). `Err(msg)`: usage or I/O
/// error, printed by `run_cli` with exit code 2.
type CmdResult = Result<u8, String>;

#[derive(Default)]
struct Opts {
    file: Option<String>,
    app: Option<String>,
    output: Option<String>,
    max_steps: Option<u64>,
    max_call_depth: Option<usize>,
}

/// Parse `[FILE] [--flag value]...`; `allowed` lists the flags this command takes.
fn parse_opts(args: &[String], allowed: &[&str]) -> Result<Opts, String> {
    let mut opts = Opts::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let flag = match arg.as_str() {
            "-o" => "--output",
            other => other,
        };
        if flag.starts_with('-') && flag.len() > 1 {
            if !allowed.contains(&flag) {
                return Err(format!("unknown option `{arg}`"));
            }
            let value = it
                .next()
                .ok_or_else(|| format!("option `{arg}` needs a value"))?;
            let number = |v: &str| -> Result<u64, String> {
                v.parse()
                    .map_err(|_| format!("option `{arg}` needs a non-negative integer, got `{v}`"))
            };
            match flag {
                "--app" => opts.app = Some(value.clone()),
                "--output" => opts.output = Some(value.clone()),
                "--max-steps" => opts.max_steps = Some(number(value)?),
                "--max-call-depth" => {
                    let n = number(value)?;
                    if n == 0 {
                        return Err("`--max-call-depth` must be at least 1".into());
                    }
                    opts.max_call_depth = Some(n as usize);
                }
                _ => unreachable!("flag listed as allowed but not handled"),
            }
        } else if opts.file.is_none() {
            opts.file = Some(arg.clone());
        } else {
            return Err(format!("unexpected argument `{arg}`"));
        }
    }
    Ok(opts)
}

/// The input file, and whether it was found through `./nex.toml`.
fn input_path(opts: &Opts) -> Result<(PathBuf, bool), String> {
    match &opts.file {
        Some(f) => Ok((PathBuf::from(f), false)),
        None if Path::new("nex.toml").is_file() => Ok((PathBuf::from("src/main.nex"), true)),
        None => {
            Err("no input file given, and there is no nex.toml in the current directory".into())
        }
    }
}

fn is_artifact(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "nexdeve")
}

fn read_source(path: &Path) -> Result<SourceFile, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_string();
    Ok(SourceFile::new(path.display().to_string(), text))
}

/// Compile a source file, printing diagnostics on failure.
fn compile(src: &SourceFile) -> Result<Module, u8> {
    nex_lang::compile_source(src).map_err(|diags| {
        for d in &diags {
            eprintln!("{}", d.render(src));
        }
        eprintln!(
            "error: could not compile `{}` due to {} error{}",
            src.name,
            diags.len(),
            if diags.len() == 1 { "" } else { "s" }
        );
        EXIT_COMPILE
    })
}

/// Load and verify a module from a `.nex` source or a `.nexdeve` artifact.
/// On failure, returns the command result to finish with.
fn load(path: &Path) -> Result<(Module, Option<SourceFile>), CmdResult> {
    if is_artifact(path) {
        let bytes =
            fs::read(path).map_err(|e| Err(format!("cannot read `{}`: {e}", path.display())))?;
        let module = artifact::decode(&bytes).map_err(|e| {
            eprintln!("error: invalid bytecode file `{}`: {e}", path.display());
            Ok(EXIT_BAD_ARTIFACT)
        })?;
        verify(&module).map_err(|e| {
            eprintln!(
                "error: bytecode verification failed for `{}`: {e}",
                path.display()
            );
            Ok(EXIT_BAD_ARTIFACT)
        })?;
        Ok((module, None))
    } else {
        let src = read_source(path).map_err(Err)?;
        let module = compile(&src).map_err(Ok)?;
        Ok((module, Some(src)))
    }
}

fn cmd_run(args: &[String]) -> CmdResult {
    let opts = parse_opts(args, &["--app", "--max-steps", "--max-call-depth"])?;
    let (path, _) = input_path(&opts)?;
    let (module, src) = match load(&path) {
        Ok(loaded) => loaded,
        Err(result) => return result,
    };
    let apps = match &opts.app {
        Some(name) => match module.find_app(name) {
            Some(i) => vec![i],
            None => {
                let names = module.app_names();
                return Err(if names.is_empty() {
                    format!("no app named `{name}`: `{}` has no apps", path.display())
                } else {
                    format!(
                        "no app named `{name}`; available apps: {}",
                        names.join(", ")
                    )
                });
            }
        },
        None => module.apps.clone(),
    };
    if apps.is_empty() {
        eprintln!(
            "error: `{}` has no `app` block to run\n  = note: add `app Main {{ ... }}` as an entry point",
            path.display()
        );
        return Ok(EXIT_COMPILE);
    }
    let mut limits = Limits {
        max_steps: opts.max_steps,
        ..Limits::default()
    };
    if let Some(depth) = opts.max_call_depth {
        limits.max_call_depth = depth;
    }

    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    for app in apps {
        if let Err(e) = vm::run_app(&module, app, &mut out, &limits) {
            let _ = out.flush();
            eprint!("{}", render_runtime_error(&e, &module, src.as_ref()));
            return Ok(EXIT_RUNTIME);
        }
    }
    if let Err(e) = out.flush() {
        eprintln!("error: failed to write output: {e}");
        return Ok(EXIT_RUNTIME);
    }
    Ok(EXIT_OK)
}

fn render_runtime_error(e: &RuntimeError, module: &Module, src: Option<&SourceFile>) -> String {
    let mut out = e.render(&module.source_name);
    if let (Some(src), Some(top)) = (src, e.trace.first()) {
        render_snippet(
            &mut out,
            src,
            top.pos.line as usize,
            top.pos.col as usize,
            1,
            None,
        );
    }
    out
}

fn cmd_check(args: &[String]) -> CmdResult {
    let opts = parse_opts(args, &[])?;
    let (path, _) = input_path(&opts)?;
    if is_artifact(&path) {
        return Err("`nex check` takes a .nex source file".into());
    }
    let src = read_source(&path)?;
    match compile(&src) {
        Ok(_) => {
            println!("ok: `{}` has no errors", src.name);
            Ok(EXIT_OK)
        }
        Err(code) => Ok(code),
    }
}

fn cmd_build(args: &[String]) -> CmdResult {
    let opts = parse_opts(args, &["--output"])?;
    let (path, in_project) = input_path(&opts)?;
    if is_artifact(&path) {
        return Err("`nex build` takes a .nex source file".into());
    }
    let src = read_source(&path)?;
    let module = match compile(&src) {
        Ok(m) => m,
        Err(code) => return Ok(code),
    };
    let out_path = match &opts.output {
        Some(o) => PathBuf::from(o),
        None if in_project => PathBuf::from("target").join("main.nexdeve"),
        None => path.with_extension("nexdeve"),
    };
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create `{}`: {e}", parent.display()))?;
    }
    let bytes = artifact::encode(&module);
    fs::write(&out_path, &bytes)
        .map_err(|e| format!("cannot write `{}`: {e}", out_path.display()))?;
    println!(
        "built `{}` ({} bytes, {} app{})",
        out_path.display(),
        bytes.len(),
        module.apps.len(),
        if module.apps.len() == 1 { "" } else { "s" }
    );
    Ok(EXIT_OK)
}

fn cmd_disasm(args: &[String]) -> CmdResult {
    let opts = parse_opts(args, &[])?;
    let (path, _) = input_path(&opts)?;
    match load(&path) {
        Ok((module, _)) => {
            print!("{}", disassemble(&module));
            Ok(EXIT_OK)
        }
        Err(result) => result,
    }
}

fn valid_project_name(name: &str) -> bool {
    name.len() <= 64
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `my-cool_app` -> `MyCoolApp`
fn app_name_for(project: &str) -> String {
    let mut cap = true;
    project
        .chars()
        .filter_map(|c| {
            if c == '-' || c == '_' {
                cap = true;
                None
            } else if cap {
                cap = false;
                Some(c.to_ascii_uppercase())
            } else {
                Some(c)
            }
        })
        .collect()
}

fn cmd_new(args: &[String]) -> CmdResult {
    let [name] = args else {
        return Err("usage: nex new <NAME>".into());
    };
    if !valid_project_name(name) {
        return Err(format!(
            "invalid project name `{name}`: use a letter followed by letters, digits, `-` or `_` (max 64)"
        ));
    }
    let root = Path::new(name);
    if root.exists() {
        return Err(format!("`{name}` already exists"));
    }
    let files = [
        (
            "nex.toml",
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nnex = \"{}\"\n",
                nex_lang::VERSION
            ),
        ),
        (
            "src/main.nex",
            format!(
                "app {} {{\n    say \"Hello from {name}!\"\n}}\n",
                app_name_for(name)
            ),
        ),
        (".gitignore", "/target\n".to_string()),
    ];
    for (rel, contents) in &files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create `{}`: {e}", parent.display()))?;
        }
        fs::write(&path, contents)
            .map_err(|e| format!("cannot write `{}`: {e}", path.display()))?;
    }
    println!("created project `{name}`\n\n    cd {name}\n    nex run");
    Ok(EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names() {
        assert!(valid_project_name("my-app_2"));
        assert!(!valid_project_name(""));
        assert!(!valid_project_name("2app"));
        assert!(!valid_project_name("a/b"));
        assert!(!valid_project_name(".."));
        assert_eq!(app_name_for("my-cool_app"), "MyCoolApp");
    }

    #[test]
    fn option_parsing() {
        let args: Vec<String> = ["main.nex", "--max-steps", "10"].map(String::from).to_vec();
        let o = parse_opts(&args, &["--max-steps"]).unwrap();
        assert_eq!(o.file.as_deref(), Some("main.nex"));
        assert_eq!(o.max_steps, Some(10));
        assert!(parse_opts(&args, &[]).is_err());
        let bad: Vec<String> = ["--max-steps", "x"].map(String::from).to_vec();
        assert!(parse_opts(&bad, &["--max-steps"]).is_err());
        let two: Vec<String> = ["a.nex", "b.nex"].map(String::from).to_vec();
        assert!(parse_opts(&two, &[]).is_err());
    }
}
