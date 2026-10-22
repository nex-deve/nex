//! Stack-based bytecode virtual machine.
//!
//! The VM is not recursive: NEX calls push frames onto a heap-allocated
//! frame stack, so deep NEX recursion cannot overflow the native stack.
//! Every resource the program controls is bounded by [`Limits`].
//!
//! Arrays and maps have value semantics. They are shared behind `Rc` and
//! copied on write (`Rc::make_mut`), so assigning a collection to another
//! variable never creates aliasing, and in-place updates of an unshared
//! local (`xs.push(..)`, `xs[i] = ..`) do not copy.

use crate::bytecode::{Builtin, Const, FnKind, Method, Module, Op, Pos};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::io::{BufRead, Write};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Rc<str>),
    Array(Rc<Vec<Value>>),
    Map(Rc<BTreeMap<Key, Value>>),
    Unit,
}

/// A map key. Maps iterate in key order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Bool(bool),
    Int(i64),
    Str(Rc<str>),
}

impl Key {
    fn from_value(v: Value) -> Result<Key, String> {
        match v {
            Value::Bool(b) => Ok(Key::Bool(b)),
            Value::Int(i) => Ok(Key::Int(i)),
            Value::Str(s) => Ok(Key::Str(s)),
            other => Err(format!(
                "{} cannot be a map key (invalid bytecode)",
                other.kind()
            )),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Key::Bool(b) => Value::Bool(*b),
            Key::Int(i) => Value::Int(*i),
            Key::Str(s) => Value::Str(s.clone()),
        }
    }
}

/// Text form of a float: always shows a decimal point for whole numbers.
fn fmt_float(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e16 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Bool(_) => "Bool",
            Value::Str(_) => "Str",
            Value::Array(_) => "array",
            Value::Map(_) => "map",
            Value::Unit => "Unit",
        }
    }

    /// Text form inside a collection: strings are quoted.
    fn write_nested(&self, f: &mut impl fmt::Write) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "{:?}", &**s),
            other => write!(f, "{other}"),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(v) => write!(f, "{v}"),
            Value::Float(v) => f.write_str(&fmt_float(*v)),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => f.write_str(s),
            Value::Unit => f.write_str("()"),
            Value::Array(items) => {
                f.write_str("[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    v.write_nested(f)?;
                }
                f.write_str("]")
            }
            Value::Map(entries) => {
                f.write_str("{")?;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    k.to_value().write_nested(f)?;
                    f.write_str(": ")?;
                    v.write_nested(f)?;
                }
                f.write_str("}")
            }
        }
    }
}

/// Resource limits for one VM run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum instructions executed; `None` means unlimited.
    pub max_steps: Option<u64>,
    /// Maximum number of active call frames (including the app frame).
    pub max_call_depth: usize,
    /// Maximum values on the shared value stack (locals + operands).
    pub max_stack_values: usize,
    /// Maximum length in bytes of any string the program builds.
    pub max_string_bytes: usize,
    /// Maximum number of elements in any array or map.
    pub max_collection_len: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_steps: None,
            max_call_depth: 1_000,
            max_stack_values: 1_000_000,
            max_string_bytes: 16 * 1024 * 1024,
            max_collection_len: 10_000_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    pub function: String,
    pub kind: FnKind,
    /// Source file that declared the function.
    pub file: String,
    pub pos: Pos,
}

/// A runtime failure. `trace[0]` is the innermost frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
    pub trace: Vec<TraceEntry>,
    /// Whether `try`/`catch` may handle it. Exceeding the step limit and
    /// invalid bytecode are never catchable.
    pub catchable: bool,
}

impl RuntimeError {
    /// Render as `runtime error: ...` plus a call trace.
    pub fn render(&self) -> String {
        let mut out = format!("runtime error: {}\n", self.message);
        for t in &self.trace {
            let kind = match t.kind {
                FnKind::Fn => "fn",
                FnKind::App => "app",
            };
            let _ = writeln!(
                out,
                "  at {kind} {} ({}:{}:{})",
                t.function, t.file, t.pos.line, t.pos.col
            );
        }
        out
    }
}

/// Everything a program may touch outside the VM.
pub struct Host<'h> {
    /// Standard output (`say`).
    pub out: &'h mut dyn Write,
    /// Standard error (`eprint`).
    pub err: &'h mut dyn Write,
    /// Standard input (`input`).
    pub input: &'h mut dyn BufRead,
    /// Values returned by `args()`.
    pub args: Vec<String>,
    /// Whether file, environment and network built-ins are allowed.
    /// `false` is the CLI's `--sandbox` mode.
    pub allow_io: bool,
    /// Open HTTP servers and connections waiting for a response.
    pub net: Net,
}

impl<'h> Host<'h> {
    pub fn new(out: &'h mut dyn Write, err: &'h mut dyn Write, input: &'h mut dyn BufRead) -> Self {
        Host {
            out,
            err,
            input,
            args: Vec::new(),
            allow_io: true,
            net: Net::default(),
        }
    }
}

/// HTTP server state owned by a [`Host`].
#[derive(Default)]
pub struct Net {
    listeners: Vec<std::net::TcpListener>,
    pending: std::collections::HashMap<String, std::net::TcpStream>,
    next_id: u64,
}

/// Largest request body `http_accept` reads.
const MAX_BODY: usize = 16 * 1024 * 1024;
/// Largest request head (request line + headers).
const MAX_HEAD: usize = 64 * 1024;

fn reason(status: i64) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "",
    }
}

struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Read one HTTP/1.1 request. `None` if it is malformed or too large.
fn read_request(stream: &mut std::net::TcpStream) -> Option<Request> {
    use std::io::Read;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8(buf[..head_end].to_vec()).ok()?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
    let mut headers = Vec::new();
    let mut length = 0usize;
    for line in lines {
        let (name, value) = line.split_once(':')?;
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim().to_string());
        if name == "content-length" {
            length = value.parse().ok()?;
        }
        headers.push((name, value));
    }
    if length > MAX_BODY {
        return None;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(Request {
        method,
        target,
        headers,
        body,
    })
}

/// Run `curl` for an HTTP(S) request and return the response body.
fn curl(name: &str, url: &str, post: Option<(&str, &str)>) -> Res<String> {
    use std::process::{Command, Stdio};
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!(
            "{name}: the URL must start with http:// or https://"
        ));
    }
    const MARK: &str = "\n@@NEX_HTTP_STATUS@@";
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-L", "--max-time", "60", "--proto", "=http,https"])
        .args(["--proto-redir", "=http,https", "-w"])
        .arg(format!("{MARK}%{{http_code}}"));
    if let Some((_, content_type)) = post {
        cmd.args(["-X", "POST", "--data-binary", "@-", "-H"])
            .arg(format!("Content-Type: {content_type}"));
    }
    cmd.arg("--url").arg(url);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        format!("{name}: needs the `curl` program, which could not be started ({e})")
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        if let Some((body, _)) = post {
            let _ = stdin.write_all(body.as_bytes());
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{name}: {e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("{name}: request to {url} failed: {msg}"));
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (body, status) = text
        .rsplit_once(MARK)
        .ok_or_else(|| format!("{name}: unexpected response from curl"))?;
    let status: i64 = status.trim().parse().unwrap_or(0);
    if status >= 400 {
        let preview: String = body.chars().take(200).collect();
        return Err(format!("{name}: {url} returned HTTP {status}: {preview}"));
    }
    Ok(body.to_string())
}

fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Unit => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) | Value::Float(_) => "number",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Map(_) => "object",
    }
}

struct Frame {
    func: usize,
    pc: usize,
    base: usize,
}

/// An active `try` block.
struct Handler {
    /// Number of frames when it was installed (it belongs to the last one).
    frames: usize,
    /// Value-stack height to restore before running the `catch` code.
    stack: usize,
    catch_pc: usize,
}

enum Flow {
    Continue,
    /// The app finished; `Some(code)` if it called `exit(code)`.
    Finished(Option<i32>),
}

/// Run one app of a module. Returns `Some(code)` if the program called
/// `exit(code)`. The module must have passed [`crate::verify::verify`];
/// unverified bytecode may produce runtime errors but never undefined
/// behaviour.
pub fn run_app(
    module: &Module,
    app: u32,
    host: &mut Host,
    limits: &Limits,
) -> Result<Option<i32>, RuntimeError> {
    let consts = module
        .consts
        .iter()
        .map(|c| match c {
            Const::Int(v) => Value::Int(*v),
            Const::Float(v) => Value::Float(*v),
            Const::Str(s) => Value::Str(Rc::from(s.as_str())),
        })
        .collect();
    let mut vm = Vm {
        m: module,
        consts,
        host,
        limits,
        stack: Vec::new(),
        frames: Vec::new(),
        handlers: Vec::new(),
        steps: 0,
    };
    match module.functions.get(app as usize) {
        Some(f) if f.kind == FnKind::App => {}
        _ => return Err(vm.error(format!("no app with index {app}"))),
    }
    vm.enter(app as usize)?;
    loop {
        match vm.step() {
            Ok(Flow::Continue) => {}
            Ok(Flow::Finished(code)) => return Ok(code),
            Err(e) => vm.recover(e)?,
        }
    }
}

struct Vm<'a, 'h> {
    m: &'a Module,
    consts: Vec<Value>,
    host: &'a mut Host<'h>,
    limits: &'a Limits,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    handlers: Vec<Handler>,
    steps: u64,
}

/// Result of a helper that does not have access to the VM; the VM turns
/// the message into a [`RuntimeError`] with a trace.
type Res<T> = Result<T, String>;

fn want_int(v: &Value) -> Res<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        other => Err(format!(
            "expected Int, found {} (invalid bytecode)",
            other.kind()
        )),
    }
}

fn want_str(v: &Value) -> Res<Rc<str>> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => Err(format!(
            "expected Str, found {} (invalid bytecode)",
            other.kind()
        )),
    }
}

/// Convert an Int index into a position in `0..len` (or `0..=len` when
/// `allow_end`), with a readable error.
fn index_in(i: i64, len: usize, allow_end: bool) -> Res<usize> {
    let max = if allow_end {
        len
    } else {
        len.saturating_sub(1)
    };
    if i < 0 || (i as u64) > max as u64 || (!allow_end && len == 0) {
        return Err(format!("index {i} is out of bounds for length {len}"));
    }
    Ok(i as usize)
}

fn slice_range(a: i64, b: i64, len: usize) -> Res<(usize, usize)> {
    if a < 0 || b < a || b as u64 > len as u64 {
        return Err(format!("slice({a}, {b}) is out of bounds for length {len}"));
    }
    Ok((a as usize, b as usize))
}

fn compare(a: &Value, b: &Value) -> Res<Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => Ok(x.total_cmp(y)),
        (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
        _ => Err(format!(
            "cannot compare {} with {} (invalid bytecode)",
            a.kind(),
            b.kind()
        )),
    }
}

fn char_index(s: &str, byte: usize) -> i64 {
    s[..byte].chars().count() as i64
}

impl Vm<'_, '_> {
    /// A runtime error at the current position. Errors caused by invalid
    /// bytecode cannot be caught.
    fn error(&self, message: impl Into<String>) -> RuntimeError {
        let message = message.into();
        let catchable = !message.contains("(invalid bytecode)");
        let trace = self
            .frames
            .iter()
            .rev()
            .map(|fr| {
                let f = &self.m.functions[fr.func];
                let pc = fr.pc.saturating_sub(1);
                TraceEntry {
                    function: f.name.clone(),
                    kind: f.kind,
                    file: self.m.source_of(f).to_string(),
                    pos: f.positions.get(pc).copied().unwrap_or_default(),
                }
            })
            .collect();
        RuntimeError {
            message,
            trace,
            catchable,
        }
    }

    /// An error that `try` cannot catch.
    fn fatal(&self, message: impl Into<String>) -> RuntimeError {
        RuntimeError {
            catchable: false,
            ..self.error(message)
        }
    }

    /// Hand `e` to the innermost `try` handler, or give it back if there is
    /// none or it cannot be caught.
    fn recover(&mut self, e: RuntimeError) -> Result<(), RuntimeError> {
        if !e.catchable {
            return Err(e);
        }
        let Some(h) = self.handlers.pop() else {
            return Err(e);
        };
        if h.frames == 0 || h.frames > self.frames.len() {
            return Err(self.fatal("corrupt handler stack (invalid bytecode)"));
        }
        self.frames.truncate(h.frames);
        self.stack.truncate(h.stack);
        self.stack.push(Value::Str(Rc::from(e.message)));
        self.frames.last_mut().expect("frame").pc = h.catch_pc;
        Ok(())
    }

    /// Built-ins that reach outside the VM.
    fn host_builtin(&mut self, b: Builtin, args: &[Value]) -> Res<Value> {
        let s = |i: usize| want_str(args.get(i).ok_or("missing argument (invalid bytecode)")?);
        let io_needed = matches!(
            b,
            Builtin::ReadFile
                | Builtin::WriteFile
                | Builtin::AppendFile
                | Builtin::FileExists
                | Builtin::ListDir
                | Builtin::DeleteFile
                | Builtin::Env
                | Builtin::HttpGet
                | Builtin::HttpPost
                | Builtin::HttpListen
                | Builtin::HttpAccept
                | Builtin::HttpRespond
        );
        if io_needed && !self.host.allow_io {
            return Err(format!(
                "{}: file, environment and network access is disabled (--sandbox)",
                b.name()
            ));
        }
        let int = |i: usize| want_int(args.get(i).ok_or("missing argument (invalid bytecode)")?);
        if matches!(b, Builtin::HttpAccept | Builtin::SleepMs) {
            // Show everything printed so far before waiting.
            let _ = self.host.out.flush();
        }
        match b {
            Builtin::HttpGet => {
                let body = curl("http_get", &s(0)?, None)?;
                self.str_value(body)
            }
            Builtin::HttpPost => {
                let (url, body, content_type) = (s(0)?, s(1)?, s(2)?);
                let reply = curl("http_post", &url, Some((&body, &content_type)))?;
                self.str_value(reply)
            }
            Builtin::HttpListen => {
                let port = int(0)?;
                if !(0..=65535).contains(&port) {
                    return Err(format!("http_listen({port}): port must be 0-65535"));
                }
                let listener = std::net::TcpListener::bind(("127.0.0.1", port as u16))
                    .map_err(|e| format!("http_listen: cannot listen on port {port}: {e}"))?;
                self.host.net.listeners.push(listener);
                Ok(Value::Int(self.host.net.listeners.len() as i64 - 1))
            }
            Builtin::HttpPort => {
                let h = int(0)?;
                let listener = usize::try_from(h)
                    .ok()
                    .and_then(|h| self.host.net.listeners.get(h))
                    .ok_or_else(|| format!("http_port: no server with handle {h}"))?;
                let port = listener
                    .local_addr()
                    .map_err(|e| format!("http_port: {e}"))?
                    .port();
                Ok(Value::Int(port as i64))
            }
            Builtin::HttpAccept => {
                let h = int(0)?;
                let listener = usize::try_from(h)
                    .ok()
                    .and_then(|h| self.host.net.listeners.get(h))
                    .ok_or_else(|| format!("http_accept: no server with handle {h}"))?;
                loop {
                    let (mut stream, _) =
                        listener.accept().map_err(|e| format!("http_accept: {e}"))?;
                    let Some(Request {
                        method,
                        target,
                        headers,
                        body,
                    }) = read_request(&mut stream)
                    else {
                        let _ = stream.write_all(
                            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                        continue;
                    };
                    let (path, query) = match target.split_once('?') {
                        Some((p, q)) => (p.to_string(), q.to_string()),
                        None => (target, String::new()),
                    };
                    self.host.net.next_id += 1;
                    let id = self.host.net.next_id.to_string();
                    let mut map = BTreeMap::new();
                    let mut put = |k: String, v: String| {
                        map.insert(Key::Str(Rc::from(k)), Value::Str(Rc::from(v)));
                    };
                    put("id".into(), id.clone());
                    put("method".into(), method);
                    put("path".into(), path);
                    put("query".into(), query);
                    put("body".into(), String::from_utf8_lossy(&body).into_owned());
                    for (name, value) in headers {
                        put(format!("header:{name}"), value);
                    }
                    self.host.net.pending.insert(id, stream);
                    return Ok(Value::Map(Rc::new(map)));
                }
            }
            Builtin::HttpRespond => {
                let (id, status, content_type, body) = (s(0)?, int(1)?, s(2)?, s(3)?);
                if !(100..=999).contains(&status) {
                    return Err(format!("http_respond: invalid status {status}"));
                }
                let mut stream =
                    self.host.net.pending.remove(&*id).ok_or_else(|| {
                        format!("http_respond: no open request with id {:?}", &*id)
                    })?;
                let head = format!(
                    "HTTP/1.1 {status} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reason(status),
                    content_type.replace(['\r', '\n'], ""),
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
                Ok(Value::Unit)
            }
            Builtin::Input => {
                let prompt = s(0)?;
                let _ = write!(self.host.out, "{prompt}");
                let _ = self.host.out.flush();
                let mut line = String::new();
                match self.host.input.read_line(&mut line) {
                    Ok(0) => Err("input: no more input (end of file)".into()),
                    Ok(_) => {
                        let trimmed = line.trim_end_matches(['\n', '\r']);
                        self.str_value(trimmed.to_string())
                    }
                    Err(e) => Err(format!("input: {e}")),
                }
            }
            Builtin::ReadFile => {
                let path = s(0)?;
                let text = std::fs::read_to_string(&*path)
                    .map_err(|e| format!("read_file: cannot read {:?}: {e}", &*path))?;
                self.str_value(text)
            }
            Builtin::WriteFile | Builtin::AppendFile => {
                let (path, text) = (s(0)?, s(1)?);
                let result = if b == Builtin::WriteFile {
                    std::fs::write(&*path, text.as_bytes())
                } else {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&*path)
                        .and_then(|mut f| f.write_all(text.as_bytes()))
                };
                result.map_err(|e| format!("{}: cannot write {:?}: {e}", b.name(), &*path))?;
                Ok(Value::Unit)
            }
            Builtin::FileExists => Ok(Value::Bool(std::path::Path::new(&*s(0)?).exists())),
            Builtin::ListDir => {
                let path = s(0)?;
                let mut names: Vec<String> = std::fs::read_dir(&*path)
                    .map_err(|e| format!("list_dir: cannot read {:?}: {e}", &*path))?
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                self.check_len(names.len())?;
                Ok(Value::Array(Rc::new(
                    names.into_iter().map(|n| Value::Str(Rc::from(n))).collect(),
                )))
            }
            Builtin::DeleteFile => {
                let path = s(0)?;
                std::fs::remove_file(&*path)
                    .map_err(|e| format!("delete_file: cannot delete {:?}: {e}", &*path))?;
                Ok(Value::Unit)
            }
            Builtin::Args => Ok(Value::Array(Rc::new(
                self.host
                    .args
                    .iter()
                    .map(|a| Value::Str(Rc::from(a.as_str())))
                    .collect(),
            ))),
            Builtin::Env => {
                let (name, default) = (s(0)?, s(1)?);
                Ok(match std::env::var(&*name) {
                    Ok(v) => Value::Str(Rc::from(v)),
                    Err(_) => Value::Str(default),
                })
            }
            Builtin::Eprint => {
                let text = s(0)?;
                writeln!(self.host.err, "{text}").map_err(|e| format!("eprint: {e}"))?;
                Ok(Value::Unit)
            }
            Builtin::TimeMs => {
                let ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                Ok(Value::Int(ms))
            }
            Builtin::SleepMs => {
                let ms = want_int(args.first().ok_or("missing argument (invalid bytecode)")?)?;
                if ms < 0 {
                    return Err(format!("sleep_ms({ms}): duration cannot be negative"));
                }
                std::thread::sleep(std::time::Duration::from_millis(ms as u64));
                Ok(Value::Unit)
            }
            other => self.builtin(other, args),
        }
    }

    fn push(&mut self, v: Value) -> Result<(), RuntimeError> {
        if self.stack.len() >= self.limits.max_stack_values {
            return Err(self.error("value stack limit exceeded"));
        }
        self.stack.push(v);
        Ok(())
    }

    /// Lowest stack index the current frame's operands may use.
    fn floor(&self) -> usize {
        self.frames.last().map_or(0, |fr| {
            fr.base + self.m.functions[fr.func].num_locals as usize
        })
    }

    fn pop(&mut self) -> Result<Value, RuntimeError> {
        if self.stack.len() <= self.floor() {
            return Err(self.error("operand stack underflow (invalid bytecode)"));
        }
        Ok(self.stack.pop().expect("checked above"))
    }

    /// Pop `n` values, returned in push order.
    fn pop_n(&mut self, n: usize) -> Result<Vec<Value>, RuntimeError> {
        if self.stack.len() < self.floor() + n {
            return Err(self.error("operand stack underflow (invalid bytecode)"));
        }
        Ok(self.stack.split_off(self.stack.len() - n))
    }

    fn pop_bool(&mut self) -> Result<bool, RuntimeError> {
        match self.pop()? {
            Value::Bool(b) => Ok(b),
            other => Err(self.error(format!(
                "expected Bool, found {} (invalid bytecode)",
                other.kind()
            ))),
        }
    }

    fn check_string(&self, len: usize) -> Res<()> {
        if len > self.limits.max_string_bytes {
            return Err(format!(
                "string length limit of {} bytes exceeded",
                self.limits.max_string_bytes
            ));
        }
        Ok(())
    }

    fn check_len(&self, len: usize) -> Res<()> {
        if len > self.limits.max_collection_len {
            return Err(format!(
                "collection size limit of {} elements exceeded",
                self.limits.max_collection_len
            ));
        }
        Ok(())
    }

    fn str_value(&self, s: String) -> Res<Value> {
        self.check_string(s.len())?;
        Ok(Value::Str(Rc::from(s)))
    }

    fn enter(&mut self, func: usize) -> Result<(), RuntimeError> {
        let Some(f) = self.m.functions.get(func) else {
            return Err(self.error(format!("function index {func} out of range")));
        };
        if self.frames.len() >= self.limits.max_call_depth {
            return Err(self.error(format!(
                "call depth limit of {} exceeded (is there infinite recursion?)",
                self.limits.max_call_depth
            )));
        }
        let arity = f.arity as usize;
        if self.stack.len() < self.floor() + arity {
            return Err(self.error("not enough arguments on the stack (invalid bytecode)"));
        }
        let extra = f.num_locals.saturating_sub(f.arity) as usize;
        if self.stack.len() + extra > self.limits.max_stack_values {
            return Err(self.error("value stack limit exceeded"));
        }
        let base = self.stack.len() - arity;
        self.stack.extend(std::iter::repeat_n(Value::Unit, extra));
        self.frames.push(Frame { func, pc: 0, base });
        Ok(())
    }

    fn arith(&self, op: Op, a: Value, b: Value) -> Res<Value> {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => {
                let (r, sym) = match op {
                    Op::Add => (a.checked_add(b), "+"),
                    Op::Sub => (a.checked_sub(b), "-"),
                    Op::Mul => (a.checked_mul(b), "*"),
                    Op::Div if b == 0 => return Err("division by zero".into()),
                    Op::Div => (a.checked_div(b), "/"),
                    Op::Rem if b == 0 => return Err("remainder by zero".into()),
                    _ => (a.checked_rem(b), "%"),
                };
                r.map(Value::Int)
                    .ok_or_else(|| format!("integer overflow in `{a} {sym} {b}`"))
            }
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(match op {
                Op::Add => a + b,
                Op::Sub => a - b,
                Op::Mul => a * b,
                Op::Div => a / b,
                _ => a % b,
            })),
            (Value::Array(a), Value::Array(b)) if op == Op::Add => {
                self.check_len(a.len() + b.len())?;
                let mut v = Vec::with_capacity(a.len() + b.len());
                v.extend(a.iter().cloned());
                v.extend(b.iter().cloned());
                Ok(Value::Array(Rc::new(v)))
            }
            (a, b) => Err(format!(
                "cannot apply {} to {} and {} (invalid bytecode)",
                op.name(),
                a.kind(),
                b.kind()
            )),
        }
    }

    fn index(&self, base: Value, index: Value) -> Res<Value> {
        match base {
            Value::Array(items) => {
                let i = index_in(want_int(&index)?, items.len(), false)?;
                Ok(items[i].clone())
            }
            Value::Map(map) => {
                let key = Key::from_value(index)?;
                map.get(&key).cloned().ok_or_else(|| {
                    let mut shown = String::new();
                    let _ = key.to_value().write_nested(&mut shown);
                    format!("key {shown} not found in map (use .get(key, default) or .has(key))")
                })
            }
            other => Err(format!(
                "cannot index into {} (invalid bytecode)",
                other.kind()
            )),
        }
    }

    fn store_path(&self, target: &mut Value, path: &[Value], value: Value) -> Res<()> {
        let (first, rest) = path.split_first().expect("non-empty path");
        match target {
            Value::Array(items) => {
                let items = Rc::make_mut(items);
                let i = index_in(want_int(first)?, items.len(), false)?;
                if rest.is_empty() {
                    items[i] = value;
                    Ok(())
                } else {
                    self.store_path(&mut items[i], rest, value)
                }
            }
            Value::Map(map) => {
                let key = Key::from_value(first.clone())?;
                if rest.is_empty() {
                    if !map.contains_key(&key) {
                        self.check_len(map.len() + 1)?;
                    }
                    Rc::make_mut(map).insert(key, value);
                    return Ok(());
                }
                let map = Rc::make_mut(map);
                match map.get_mut(&key) {
                    Some(inner) => self.store_path(inner, rest, value),
                    None => {
                        let mut shown = String::new();
                        let _ = key.to_value().write_nested(&mut shown);
                        Err(format!("key {shown} not found in map"))
                    }
                }
            }
            other => Err(format!(
                "cannot index into {} (invalid bytecode)",
                other.kind()
            )),
        }
    }

    fn method(&self, m: Method, recv: Value, args: &[Value]) -> Res<Value> {
        let arg = |i: usize| {
            args.get(i)
                .cloned()
                .ok_or("missing argument (invalid bytecode)")
        };
        match (m, recv) {
            (Method::Len, Value::Array(a)) => Ok(Value::Int(a.len() as i64)),
            (Method::Len, Value::Map(map)) => Ok(Value::Int(map.len() as i64)),
            (Method::Len, Value::Str(s)) => Ok(Value::Int(s.chars().count() as i64)),
            (Method::Contains, Value::Array(a)) => {
                let x = arg(0)?;
                Ok(Value::Bool(a.contains(&x)))
            }
            (Method::Contains, Value::Str(s)) => Ok(Value::Bool(s.contains(&*want_str(&arg(0)?)?))),
            (Method::IndexOf, Value::Array(a)) => {
                let x = arg(0)?;
                Ok(Value::Int(
                    a.iter().position(|v| *v == x).map_or(-1, |i| i as i64),
                ))
            }
            (Method::IndexOf, Value::Str(s)) => {
                let sub = want_str(&arg(0)?)?;
                Ok(Value::Int(s.find(&*sub).map_or(-1, |b| char_index(&s, b))))
            }
            (Method::Join, Value::Array(a)) => {
                let sep = want_str(&arg(0)?)?;
                let mut out = String::new();
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push_str(&sep);
                    }
                    let _ = write!(out, "{v}");
                    self.check_string(out.len())?;
                }
                self.str_value(out)
            }
            (Method::Sorted, Value::Array(a)) => {
                let mut v = (*a).clone();
                let mut failure = None;
                v.sort_by(|x, y| {
                    compare(x, y).unwrap_or_else(|e| {
                        failure = Some(e);
                        Ordering::Equal
                    })
                });
                match failure {
                    Some(e) => Err(e),
                    None => Ok(Value::Array(Rc::new(v))),
                }
            }
            (Method::Reversed, Value::Array(a)) => {
                let mut v = (*a).clone();
                v.reverse();
                Ok(Value::Array(Rc::new(v)))
            }
            (Method::Slice, Value::Array(a)) => {
                let (s, e) = slice_range(want_int(&arg(0)?)?, want_int(&arg(1)?)?, a.len())?;
                Ok(Value::Array(Rc::new(a[s..e].to_vec())))
            }
            (Method::Slice, Value::Str(st)) => {
                let len = st.chars().count();
                let (s, e) = slice_range(want_int(&arg(0)?)?, want_int(&arg(1)?)?, len)?;
                self.str_value(st.chars().skip(s).take(e - s).collect())
            }
            (Method::Upper, Value::Str(s)) => self.str_value(s.to_uppercase()),
            (Method::Lower, Value::Str(s)) => self.str_value(s.to_lowercase()),
            (Method::Trim, Value::Str(s)) => self.str_value(s.trim().to_string()),
            (Method::StartsWith, Value::Str(s)) => {
                Ok(Value::Bool(s.starts_with(&*want_str(&arg(0)?)?)))
            }
            (Method::EndsWith, Value::Str(s)) => {
                Ok(Value::Bool(s.ends_with(&*want_str(&arg(0)?)?)))
            }
            (Method::Split, Value::Str(s)) => {
                let sep = want_str(&arg(0)?)?;
                if sep.is_empty() {
                    return Err("split separator must not be empty (use .chars())".into());
                }
                let parts: Vec<Value> = s.split(&*sep).map(|p| Value::Str(Rc::from(p))).collect();
                self.check_len(parts.len())?;
                Ok(Value::Array(Rc::new(parts)))
            }
            (Method::Chars, Value::Str(s)) => {
                let chars: Vec<Value> = s
                    .chars()
                    .map(|c| Value::Str(Rc::from(c.to_string())))
                    .collect();
                self.check_len(chars.len())?;
                Ok(Value::Array(Rc::new(chars)))
            }
            (Method::Replace, Value::Str(s)) => {
                let from = want_str(&arg(0)?)?;
                let to = want_str(&arg(1)?)?;
                if from.is_empty() {
                    return Err("replace: the text to find must not be empty".into());
                }
                let count = s.matches(&*from).count();
                let new_len = s.len() - count * from.len() + count * to.len();
                self.check_string(new_len)?;
                self.str_value(s.replace(&*from, &to))
            }
            (Method::Has, Value::Map(map)) => {
                Ok(Value::Bool(map.contains_key(&Key::from_value(arg(0)?)?)))
            }
            (Method::Get, Value::Map(map)) => {
                let key = Key::from_value(arg(0)?)?;
                Ok(map.get(&key).cloned().unwrap_or(arg(1)?))
            }
            (Method::Keys, Value::Map(map)) => Ok(Value::Array(Rc::new(
                map.keys().map(Key::to_value).collect(),
            ))),
            (Method::Values, Value::Map(map)) => {
                Ok(Value::Array(Rc::new(map.values().cloned().collect())))
            }
            // Json values: objects are maps, arrays are arrays, null is Unit.
            (Method::JsonGet, Value::Map(map)) => {
                let key = Key::from_value(arg(0)?)?;
                Ok(map.get(&key).cloned().unwrap_or(Value::Unit))
            }
            (Method::JsonHas, Value::Map(map)) => {
                Ok(Value::Bool(map.contains_key(&Key::from_value(arg(0)?)?)))
            }
            (Method::JsonHas, _) => Ok(Value::Bool(false)),
            (Method::JsonKeys, Value::Map(map)) => Ok(Value::Array(Rc::new(
                map.keys().map(Key::to_value).collect(),
            ))),
            (Method::JsonAt, Value::Array(items)) => {
                let i = index_in(want_int(&arg(0)?)?, items.len(), false)?;
                Ok(items[i].clone())
            }
            (Method::JsonItems, Value::Array(items)) => Ok(Value::Array(items)),
            (Method::JsonStr, Value::Str(s)) => Ok(Value::Str(s)),
            (Method::JsonInt, Value::Int(n)) => Ok(Value::Int(n)),
            (Method::JsonInt, Value::Float(x))
                if x.fract() == 0.0 && x >= i64::MIN as f64 && x < -(i64::MIN as f64) =>
            {
                Ok(Value::Int(x as i64))
            }
            (Method::JsonFloat, Value::Float(x)) => Ok(Value::Float(x)),
            (Method::JsonFloat, Value::Int(n)) => Ok(Value::Float(n as f64)),
            (Method::JsonBool, Value::Bool(b)) => Ok(Value::Bool(b)),
            (Method::JsonIsNull, v) => Ok(Value::Bool(v == Value::Unit)),
            (Method::JsonKind, v) => Ok(Value::Str(Rc::from(json_kind(&v)))),
            (
                m @ (Method::JsonGet
                | Method::JsonKeys
                | Method::JsonAt
                | Method::JsonItems
                | Method::JsonStr
                | Method::JsonInt
                | Method::JsonFloat
                | Method::JsonBool),
                v,
            ) => {
                let wanted = match m {
                    Method::JsonGet | Method::JsonKeys => "an object",
                    Method::JsonAt | Method::JsonItems => "an array",
                    Method::JsonStr => "a string",
                    Method::JsonInt => "a whole number",
                    Method::JsonFloat => "a number",
                    _ => "a bool",
                };
                Err(format!(
                    "json: {}() needs {wanted}, found {}",
                    m.name(),
                    json_kind(&v)
                ))
            }
            (Method::Len, Value::Unit) => Err("json: len() of null".into()),
            (m, recv) => Err(format!(
                "{} has no method `{}` (invalid bytecode)",
                recv.kind(),
                m.name()
            )),
        }
    }

    fn method_mut(&self, m: Method, target: &mut Value, args: &[Value]) -> Res<Value> {
        let arg = |i: usize| {
            args.get(i)
                .cloned()
                .ok_or("missing argument (invalid bytecode)")
        };
        match (m, target) {
            (Method::Push, Value::Array(a)) => {
                self.check_len(a.len() + 1)?;
                Rc::make_mut(a).push(arg(0)?);
                Ok(Value::Unit)
            }
            (Method::Pop, Value::Array(a)) => Rc::make_mut(a)
                .pop()
                .ok_or_else(|| "pop() on an empty array".to_string()),
            (Method::Insert, Value::Array(a)) => {
                let i = index_in(want_int(&arg(0)?)?, a.len(), true)?;
                self.check_len(a.len() + 1)?;
                Rc::make_mut(a).insert(i, arg(1)?);
                Ok(Value::Unit)
            }
            (Method::RemoveAt, Value::Array(a)) => {
                let i = index_in(want_int(&arg(0)?)?, a.len(), false)?;
                Ok(Rc::make_mut(a).remove(i))
            }
            (Method::Clear, Value::Array(a)) => {
                Rc::make_mut(a).clear();
                Ok(Value::Unit)
            }
            (Method::Remove, Value::Map(map)) => {
                let key = Key::from_value(arg(0)?)?;
                Ok(Value::Bool(Rc::make_mut(map).remove(&key).is_some()))
            }
            (Method::Clear, Value::Map(map)) => {
                Rc::make_mut(map).clear();
                Ok(Value::Unit)
            }
            (m, target) => Err(format!(
                "{} has no method `{}` (invalid bytecode)",
                target.kind(),
                m.name()
            )),
        }
    }

    fn builtin(&self, b: Builtin, args: &[Value]) -> Res<Value> {
        let float = |i: usize| match args.get(i) {
            Some(Value::Float(x)) => Ok(*x),
            _ => Err("expected Float argument (invalid bytecode)".to_string()),
        };
        match b {
            Builtin::ToFloat => Ok(Value::Float(
                want_int(args.first().ok_or("missing argument")?)? as f64,
            )),
            Builtin::ToInt => {
                let x = float(0)?;
                // Int covers [-2^63, 2^63); both bounds are exact in f64.
                let min = i64::MIN as f64;
                if !x.is_finite() || x < min || x >= -min {
                    return Err(format!("to_int({}) is out of range for Int", fmt_float(x)));
                }
                Ok(Value::Int(x.trunc() as i64))
            }
            Builtin::Sqrt => Ok(Value::Float(float(0)?.sqrt())),
            Builtin::Floor => Ok(Value::Float(float(0)?.floor())),
            Builtin::Round => Ok(Value::Float(float(0)?.round())),
            Builtin::ParseInt => {
                let s = want_str(args.first().ok_or("missing argument")?)?;
                s.trim()
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| format!("parse_int: {:?} is not a valid Int", &*s))
            }
            Builtin::ParseFloat => {
                let s = want_str(args.first().ok_or("missing argument")?)?;
                match s.trim().parse::<f64>() {
                    Ok(x) if x.is_finite() => Ok(Value::Float(x)),
                    _ => Err(format!("parse_float: {:?} is not a valid Float", &*s)),
                }
            }
            Builtin::JsonParse => {
                let s = want_str(args.first().ok_or("missing argument")?)?;
                crate::json::parse(&s)
            }
            Builtin::ToJson => {
                let text = crate::json::stringify(args.first().ok_or("missing argument")?)?;
                self.str_value(text)
            }
            Builtin::Json => Ok(args.first().ok_or("missing argument")?.clone()),
            other => Err(format!(
                "{} is not a pure built-in (invalid bytecode)",
                other.name()
            )),
        }
    }

    /// Execute one instruction.
    fn step(&mut self) -> Result<Flow, RuntimeError> {
        let m = self.m;
        {
            let frame = self.frames.last_mut().expect("run with no frame");
            let (func, pc, base) = (frame.func, frame.pc, frame.base);
            let f = &m.functions[func];
            let Some(&op) = f.code.get(pc) else {
                return Err(
                    self.error("execution ran past the end of a function (invalid bytecode)")
                );
            };
            frame.pc = pc + 1;

            self.steps += 1;
            if let Some(max) = self.limits.max_steps {
                if self.steps > max {
                    return Err(self.fatal(format!("step limit of {max} instructions exceeded")));
                }
            }

            // Helpers return `Res`; attach the trace here.
            macro_rules! vm_try {
                ($e:expr) => {
                    match $e {
                        Ok(v) => v,
                        Err(msg) => return Err(self.error(msg)),
                    }
                };
            }

            match op {
                Op::PushConst(i) => {
                    let Some(v) = self.consts.get(i as usize).cloned() else {
                        return Err(self.error("constant index out of range (invalid bytecode)"));
                    };
                    self.push(v)?;
                }
                Op::PushUnit => self.push(Value::Unit)?,
                Op::PushTrue => self.push(Value::Bool(true))?,
                Op::PushFalse => self.push(Value::Bool(false))?,
                Op::Load(i) => {
                    if i >= f.num_locals {
                        return Err(self.error("local slot out of range (invalid bytecode)"));
                    }
                    let v = self.stack[base + i as usize].clone();
                    self.push(v)?;
                }
                Op::Store(i) => {
                    if i >= f.num_locals {
                        return Err(self.error("local slot out of range (invalid bytecode)"));
                    }
                    let v = self.pop()?;
                    self.stack[base + i as usize] = v;
                }
                Op::Pop => {
                    self.pop()?;
                }
                Op::MakeArray(n) => {
                    let items = self.pop_n(n as usize)?;
                    vm_try!(self.check_len(items.len()));
                    self.push(Value::Array(Rc::new(items)))?;
                }
                Op::MakeMap(n) => {
                    let flat = self.pop_n(2 * n as usize)?;
                    let mut map = BTreeMap::new();
                    let mut it = flat.into_iter();
                    while let (Some(k), Some(v)) = (it.next(), it.next()) {
                        map.insert(vm_try!(Key::from_value(k)), v);
                    }
                    self.push(Value::Map(Rc::new(map)))?;
                }
                Op::Index => {
                    let index = self.pop()?;
                    let container = self.pop()?;
                    let v = vm_try!(self.index(container, index));
                    self.push(v)?;
                }
                Op::StoreIndex(slot, depth) => {
                    if slot >= f.num_locals || depth == 0 {
                        return Err(self.error("invalid StoreIndex (invalid bytecode)"));
                    }
                    let value = self.pop()?;
                    let path = self.pop_n(depth as usize)?;
                    let at = base + slot as usize;
                    let mut target = std::mem::replace(&mut self.stack[at], Value::Unit);
                    let result = self.store_path(&mut target, &path, value);
                    self.stack[at] = target;
                    vm_try!(result);
                }
                Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let v = vm_try!(self.arith(op, a, b));
                    self.push(v)?;
                }
                Op::Neg => {
                    let v = match self.pop()? {
                        Value::Int(a) => match a.checked_neg() {
                            Some(v) => Value::Int(v),
                            None => return Err(self.error(format!("integer overflow in `-({a})`"))),
                        },
                        Value::Float(x) => Value::Float(-x),
                        other => {
                            return Err(self.error(format!(
                                "cannot negate {} (invalid bytecode)",
                                other.kind()
                            )))
                        }
                    };
                    self.push(v)?;
                }
                Op::Not => {
                    let b = self.pop_bool()?;
                    self.push(Value::Bool(!b))?;
                }
                Op::Eq | Op::Ne => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let equal = a == b;
                    self.push(Value::Bool(if op == Op::Eq { equal } else { !equal }))?;
                }
                Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    // NaN compares false with everything, as in IEEE 754.
                    let r = match (&a, &b) {
                        (Value::Float(x), Value::Float(y)) => match op {
                            Op::Lt => x < y,
                            Op::Le => x <= y,
                            Op::Gt => x > y,
                            _ => x >= y,
                        },
                        _ => {
                            let ord = vm_try!(compare(&a, &b));
                            match op {
                                Op::Lt => ord.is_lt(),
                                Op::Le => ord.is_le(),
                                Op::Gt => ord.is_gt(),
                                _ => ord.is_ge(),
                            }
                        }
                    };
                    self.push(Value::Bool(r))?;
                }
                Op::Concat => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let (a, b) = (vm_try!(want_str(&a)), vm_try!(want_str(&b)));
                    let mut s = String::with_capacity(a.len() + b.len());
                    s.push_str(&a);
                    s.push_str(&b);
                    let v = vm_try!(self.str_value(s));
                    self.push(v)?;
                }
                Op::Format(n) => {
                    let parts = self.pop_n(n as usize)?;
                    let mut s = String::new();
                    for p in &parts {
                        let _ = write!(s, "{p}");
                        vm_try!(self.check_string(s.len()));
                    }
                    self.push(Value::Str(Rc::from(s)))?;
                }
                Op::Jump(t) => self.frames.last_mut().expect("frame").pc = t as usize,
                Op::JumpIfFalse(t) => {
                    if !self.pop_bool()? {
                        self.frames.last_mut().expect("frame").pc = t as usize;
                    }
                }
                Op::Call(i) => {
                    if m.functions.get(i as usize).map(|c| c.kind) != Some(FnKind::Fn) {
                        return Err(self.error("call to a non-function (invalid bytecode)"));
                    }
                    self.enter(i as usize)?;
                }
                Op::Return => {
                    let v = self.pop()?;
                    let done = self.frames.pop().expect("frame");
                    self.stack.truncate(done.base);
                    // Handlers of the finished frame (from a `return` inside
                    // `try`) no longer apply.
                    while self
                        .handlers
                        .last()
                        .is_some_and(|h| h.frames > self.frames.len())
                    {
                        self.handlers.pop();
                    }
                    if self.frames.is_empty() {
                        return Ok(Flow::Finished(None));
                    }
                    self.push(v)?;
                }
                Op::TryBegin(t) => {
                    self.handlers.push(Handler {
                        frames: self.frames.len(),
                        stack: self.stack.len(),
                        catch_pc: t as usize,
                    });
                }
                Op::TryEnd => {
                    if self
                        .handlers
                        .last()
                        .is_none_or(|h| h.frames != self.frames.len())
                    {
                        return Err(self.error("TryEnd without a handler (invalid bytecode)"));
                    }
                    self.handlers.pop();
                }
                Op::Fail => {
                    let v = self.pop()?;
                    let message = vm_try!(want_str(&v));
                    return Err(self.error(message.to_string()));
                }
                Op::Say => {
                    let v = self.pop()?;
                    if let Err(e) = writeln!(self.host.out, "{v}") {
                        return Err(self.error(format!("failed to write output: {e}")));
                    }
                }
                Op::Method(id) => {
                    let Some(mt) = Method::from_id(id).filter(|mt| !mt.mutating()) else {
                        return Err(self.error("invalid method (invalid bytecode)"));
                    };
                    let args = self.pop_n(mt.argc() as usize)?;
                    let recv = self.pop()?;
                    let v = vm_try!(self.method(mt, recv, &args));
                    self.push(v)?;
                }
                Op::MethodMut(slot, id) => {
                    let Some(mt) = Method::from_id(id).filter(|mt| mt.mutating()) else {
                        return Err(self.error("invalid method (invalid bytecode)"));
                    };
                    if slot >= f.num_locals {
                        return Err(self.error("local slot out of range (invalid bytecode)"));
                    }
                    let args = self.pop_n(mt.argc() as usize)?;
                    let at = base + slot as usize;
                    let mut target = std::mem::replace(&mut self.stack[at], Value::Unit);
                    let result = self.method_mut(mt, &mut target, &args);
                    self.stack[at] = target;
                    let v = vm_try!(result);
                    self.push(v)?;
                }
                Op::Builtin(id) => {
                    let Some(b) = Builtin::from_id(id) else {
                        return Err(self.error("invalid built-in (invalid bytecode)"));
                    };
                    let args = self.pop_n(b.argc() as usize)?;
                    if b == Builtin::Exit {
                        let code = vm_try!(want_int(&args[0]));
                        if !(0..=255).contains(&code) {
                            return Err(self.error(format!(
                                "exit({code}): the exit code must be between 0 and 255"
                            )));
                        }
                        let _ = self.host.out.flush();
                        return Ok(Flow::Finished(Some(code as i32)));
                    }
                    let v = vm_try!(self.host_builtin(b, &args));
                    self.push(v)?;
                }
            }
        }
        Ok(Flow::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_text_forms() {
        let arr = Value::Array(Rc::new(vec![
            Value::Int(1),
            Value::Str(Rc::from("a\"b")),
            Value::Float(2.0),
        ]));
        assert_eq!(arr.to_string(), r#"[1, "a\"b", 2.0]"#);
        let mut m = BTreeMap::new();
        m.insert(Key::Str(Rc::from("b")), Value::Bool(true));
        m.insert(Key::Str(Rc::from("a")), Value::Float(0.5));
        assert_eq!(
            Value::Map(Rc::new(m)).to_string(),
            r#"{"a": 0.5, "b": true}"#
        );
        assert_eq!(Value::Float(0.1 + 0.2).to_string(), "0.30000000000000004");
        assert_eq!(Value::Float(1e20).to_string(), "100000000000000000000");
        assert_eq!(Value::Float(f64::NAN).to_string(), "NaN");
        assert_eq!(Value::Float(-3.0).to_string(), "-3.0");
    }

    #[test]
    fn index_bounds() {
        assert_eq!(index_in(0, 1, false), Ok(0));
        assert!(index_in(1, 1, false).is_err());
        assert!(index_in(-1, 5, false).is_err());
        assert!(index_in(0, 0, false).is_err());
        assert_eq!(index_in(0, 0, true), Ok(0));
        assert_eq!(index_in(3, 3, true), Ok(3));
        assert!(slice_range(2, 1, 5).is_err());
        assert_eq!(slice_range(1, 5, 5), Ok((1, 5)));
    }
}
