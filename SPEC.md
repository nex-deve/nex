# NEX Language Specification

Version: **0.1.0-alpha.1** (draft). This document describes what the
implementation in this repository actually accepts. Anything not described
here is rejected with a compile error rather than silently ignored.

## 1. Source files

- A NEX source file is UTF-8 text with the extension `.nex`. A leading
  byte-order mark is ignored by the CLI.
- Line endings may be `\n` or `\r\n`.
- Positions in diagnostics are 1-based `line:column`. Columns count Unicode
  scalar values (not bytes, not grapheme clusters).
- Identifiers are ASCII only: a letter or `_`, then letters, digits or `_`.
  String literals and comments may contain any Unicode text.

## 2. Lexical structure

### Comments

```
// line comment, to end of line
/* block comment; may span lines; does not nest */
```

### Keywords

`app fn let var say return if else while for in break continue true false use try catch fail`

Reserved for future use (rejected as identifiers):
`const struct enum match loop pub mod import nil`

### Literals

| Kind  | Syntax                                    | Notes                                     |
|-------|-------------------------------------------|-------------------------------------------|
| Int   | `0`, `42`, `1_000_000`                    | decimal; `_` separators; must fit `i64`   |
| Float | `1.5`, `0.25`, `2e8`, `1.5e-3`, `1_000.5` | digits on both sides of `.`; finite `f64` |
| Bool  | `true`, `false`                           |                                           |
| Str   | `"text"`                                  | single line; see escapes below            |
| Array | `[1, 2, 3]`, `[]`                         | trailing comma allowed                    |
| Map   | `{"a": 1, "b": 2}`, `{}`                  | trailing comma allowed; may span lines    |

There is no negative literal: `-5` is unary minus applied to `5`. As a
consequence the literal `9223372036854775808` is rejected; write
`-9223372036854775807 - 1` for the minimum `Int`. `1.` and `.5` are not
valid floats; `1..5` is a range (see `for`).

An empty `[]` or `{}` needs a known type from context, such as an
annotation (`let xs: [Int] = []`), a parameter type or the other side of
an assignment.

### String escapes and interpolation

Escapes: `\n \t \r \0 \\ \" \{ \}`. Any other escape is an error.

`{expr}` inside a string evaluates `expr` and inserts its text form:

```
say "total: {a + b}, first: {items[0]}"
```

- Any expression whose type is not `Unit` may be interpolated.
- The expression may contain string literals (`"{names.join(", ")}"`) but
  may not span lines.
- `{}` (empty) is an error. Write `\{` for a literal `{`. A lone `}` is
  literal text.

### Statement separators

Statements end at a newline or `;`. Newlines are ignored inside `(...)` and
`[...]`, after a binary operator, and between the entries of a map
literal, so long expressions can be split:

```
let total = first +
    second
```

## 3. Grammar

EBNF. `NL` is a newline token; `SEP` is `NL` or `;`.

```
program    = { SEP } { item { SEP } } EOF ;
item       = use_decl | fn_decl | app_decl ;

use_decl   = "use" IDENT { "." IDENT } ;
fn_decl    = "fn" IDENT "(" [ param { "," param } [ "," ] ] ")" [ "->" type ] block ;
param      = IDENT ":" type ;
app_decl   = "app" IDENT block ;
type       = IDENT                          (* Int | Float | Bool | Str | Unit *)
           | "[" type "]"
           | "{" type ":" type "}" ;

block      = "{" { SEP } [ stmt { SEP { SEP } stmt } ] { SEP } "}" ;
stmt       = ("let" | "var") IDENT [ ":" type ] "=" expr
           | place "=" expr
           | "say" expr
           | "return" [ expr ]
           | if_stmt
           | "while" expr block
           | "for" IDENT "in" ( expr ".." expr | expr ) block
           | "try" block { NL } "catch" IDENT block
           | "fail" expr
           | "break" | "continue"
           | block
           | expr ;
place      = IDENT { "[" expr "]" } ;
if_stmt    = "if" expr block [ { NL } "else" ( if_stmt | block ) ] ;

expr       = or ;
or         = and { "||" and } ;
and        = equality { "&&" equality } ;
equality   = compare { ("==" | "!=") compare } ;
compare    = additive { ("<" | "<=" | ">" | ">=") additive } ;
additive   = term { ("+" | "-") term } ;
term       = unary { ("*" | "/" | "%") unary } ;
unary      = ("-" | "!") unary | postfix ;
postfix    = primary { "[" expr "]" | "." IDENT "(" [ args ] ")" } ;
primary    = INT | FLOAT | "true" | "false" | STRING
           | IDENT [ "(" [ args ] ")" ]
           | "[" [ expr { "," expr } [ "," ] ] "]"
           | "{" [ entry { "," entry } [ "," ] ] "}"
           | "(" expr ")" ;
entry      = expr ":" expr ;
args       = expr { "," expr } [ "," ] ;
```

All binary operators are left-associative. Chained comparisons such as
`a < b < c` parse but are type errors. A statement starting with `{` is a
block, not a map literal.

## 4. Program structure

- **Top level** contains only `use`, `fn` and `app` declarations. Top-level
  statements are an error.
- **`app Name { ... }`** declares an *entry point*. A file may declare any
  number of apps. Apps are not functions: they take no parameters, cannot
  be called, and cannot `return` a value (a bare `return` leaves the app).
- **Running a file** (`nex run`) executes every app in declaration order.
  `nex run --app Name` executes only that app. A file with no apps passes
  `nex check` but `nex run` reports an error.
- **Functions** may be declared before or after their use, and may be
  recursive or mutually recursive. Functions cannot see the locals of apps
  or other functions (there are no closures or globals).
- Functions and apps share one namespace; duplicate names are an error.
  Built-in function names (§10) cannot be redefined.

## 5. Variables and scopes

- `let name = expr` declares an immutable binding. `var name = expr`
  declares a mutable one.
- Assignment `name = expr` and element assignment `name[i] = expr`,
  `grid[i][j] = expr` require `var`. Methods that change their receiver
  (§9) also require a `var` variable as receiver.
- A type annotation is optional: `let n: Int = 5`. Without one, the type is
  inferred from the initializer.
- Every block `{ ... }` opens a scope. A name may be redeclared in an inner
  scope (shadowing), but not twice in the same scope. Function parameters
  and `for` loop variables are immutable.
- Variables cannot hold `Unit`.

## 6. Types

| Type      | Values                                  | Text form (`say`, interpolation) |
|-----------|-----------------------------------------|----------------------------------|
| `Int`     | 64-bit signed integers                  | `-42`                            |
| `Float`   | 64-bit IEEE 754 floating point          | `2.5`, `3.0`, `inf`, `NaN`       |
| `Bool`    | `true`, `false`                         | `true` / `false`                 |
| `Str`     | immutable Unicode strings               | the text itself                  |
| `[T]`     | ordered, growable array of `T`          | `[1, 2]`, `["a", "b"]`           |
| `{K: V}`  | map from `K` to `V`; K is Int, Bool or Str | `{"a": 1}` (sorted by key)    |
| `Unit`    | result of a function without `-> T`     | not printable                    |

Typing is static and there are **no implicit conversions**; in particular
`Int` and `Float` never mix (use `to_float` / `to_int`). Strings inside an
array or map are shown quoted. Whole floats always show a decimal point.

### Value semantics

Arrays and maps behave like values: `var b = a` makes an independent copy
(internally shared until one side changes). Changing `b` never changes
`a`, and passing a collection to a function cannot change the caller's
variable.

### Operators

| Expression                          | Operand types              | Result |
|-------------------------------------|----------------------------|--------|
| `a + b`                             | Int/Int, Float/Float       | same   |
| `a + b`                             | Str/Str                    | Str    |
| `a + b`                             | [T]/[T]                    | [T] (concatenation) |
| `a - b`, `a * b`, `a / b`, `a % b`  | Int/Int, Float/Float       | same   |
| `a < b`, `<=`, `>`, `>=`            | Int, Float or Str (same)   | Bool   |
| `a == b`, `a != b`                  | same type, no Unit inside  | Bool   |
| `a && b`, `a \|\| b`                | Bool, Bool                 | Bool   |
| `-a`                                | Int or Float               | same   |
| `!a`                                | Bool                       | Bool   |
| `a[i]`                              | [T] with Int; {K: V} with K | T / V |
| `"..{e}.."`                         | any non-Unit               | Str    |

Strings compare by Unicode code point order. `==` on arrays and maps
compares contents. Conditions of `if` and `while` must be `Bool`. `&&` and
`||` short-circuit.

## 7. Functions and returns

- Parameters must be annotated. The return type defaults to `Unit`.
- A function with a non-Unit return type must end in `return <value>` on
  every path. The check is syntactic: an `if` counts only when it has an
  `else` and both branches return; loops never count, even `while true`.
- A `Unit` function may use bare `return`, and may not return a value.
- Arguments are evaluated left to right, then the call is made.

## 8. Loops

- `while cond { ... }` repeats while `cond` is true.
- `for i in a..b { ... }` runs with `i = a, a+1, ..., b-1` (empty when
  `a >= b`). `a` and `b` are evaluated once.
- `for x in array { ... }` visits each element in order. The array is
  captured when the loop starts; changing the variable inside the loop
  does not change what is visited.
- `for k in map { ... }` visits the keys in sorted order.
- `break` leaves the innermost loop; `continue` starts its next iteration.

## 9. Methods

Called as `value.method(args)`. Methods marked **(mut)** change their
receiver, which must be a `var` variable.

**Arrays `[T]`**

| Method | Result | Notes |
|---|---|---|
| `len()` | Int | |
| `push(x)` | Unit | (mut) append |
| `pop()` | T | (mut) remove last; runtime error if empty |
| `insert(i, x)` | Unit | (mut) `0 <= i <= len` |
| `remove_at(i)` | T | (mut) |
| `clear()` | Unit | (mut) |
| `contains(x)` | Bool | |
| `index_of(x)` | Int | first index, or -1 |
| `join(sep)` | Str | text forms joined with `sep` |
| `sorted()` | [T] | T must be Int, Float or Str |
| `reversed()` | [T] | |
| `slice(a, b)` | [T] | elements `a..b`; `0 <= a <= b <= len` |

**Strings `Str`** (indices count Unicode scalar values)

| Method | Result |
|---|---|
| `len()` | Int |
| `upper()`, `lower()`, `trim()` | Str |
| `contains(s)`, `starts_with(s)`, `ends_with(s)` | Bool |
| `index_of(s)` | Int (-1 if absent) |
| `split(sep)` | [Str] (`sep` must not be empty) |
| `chars()` | [Str] |
| `replace(from, to)` | Str (`from` must not be empty) |
| `slice(a, b)` | Str |

Strings cannot be indexed with `[]`; use `chars()` or `slice`.

**Maps `{K: V}`**

| Method | Result | Notes |
|---|---|---|
| `len()` | Int | |
| `has(k)` | Bool | |
| `get(k, default)` | V | `default` if absent |
| `keys()` | [K] | sorted |
| `values()` | [V] | in key order |
| `remove(k)` | Bool | (mut) whether it was present |
| `clear()` | Unit | (mut) |

`m[k]` on a missing key is a runtime error; `m[k] = v` inserts or replaces.

## 10. Built-in functions

| Function | Type | Notes |
|---|---|---|
| `to_float(n)` | Int → Float | |
| `to_int(x)` | Float → Int | truncates toward zero; runtime error if NaN, infinite or out of range |
| `sqrt(x)`, `floor(x)`, `round(x)` | Float → Float | `round` rounds half away from zero |
| `parse_int(s)` | Str → Int | surrounding spaces allowed; runtime error if invalid |
| `parse_float(s)` | Str → Float | runtime error if invalid or not finite |

## 11. Numeric semantics

- `Int` is two's-complement 64-bit. Overflow in `+ - * / %` or unary `-` is
  a **runtime error**; it never wraps. Division truncates toward zero; `%`
  has the sign of the left operand (`-7 / 2 == -3`, `-7 % 2 == -1`).
  Division or remainder by zero is a runtime error.
- `Float` follows IEEE 754 double precision: division by zero gives `inf`
  or `NaN` instead of an error, and `NaN != NaN`. `<` and friends return
  false when either side is NaN. `sorted()` uses IEEE total order.

## 12. Modules and imports

```
use lib.math        // loads lib/math.nex next to the main file
use text            // loads text.nex

app Main {
    say math.square(4)
    say text.shout("hi")
}
```

- `use a.b.c` loads `a/b/c.nex`, relative to the directory of the main
  file. Its functions are called with the last path segment as prefix:
  `c.name(...)`. Two imports with the same last segment are an error.
- A module contains only `fn` (and `use`) declarations; an `app` in a
  module is an error. Functions inside a module call each other by plain
  name.
- Every module is loaded once, however many files import it. Because
  modules contain only declarations, circular imports are allowed.
- A missing file is a compile error: `unresolved module `a.b``. There is
  no standard-library module yet (`net.http` is planned).
- Errors in a module are reported against that module's file.

## 13. Errors

- **Compile errors** (syntax, names, types) are reported with file, line,
  column and a source snippet. The parser stops at the first syntax error
  in each file; semantic analysis reports every error it finds. A program
  with any compile error never runs.
- **Runtime errors** (overflow, division by zero, index out of bounds,
  missing map key, invalid parse, failed I/O, `fail`, resource limits)
  stop the program unless caught. Output already produced remains. The
  error shows a call trace with source positions.

### `fail`, `try` and `catch`

```
fn withdraw(balance: Int, amount: Int) -> Int {
    if amount > balance {
        fail "not enough money"
    }
    return balance - amount
}

try {
    say withdraw(10, 50)
} catch err {
    say "refused: {err}"      // err is the message, a Str
}
```

- `fail message` raises an error with a `Str` message. It ends the
  current path like `return` (it counts as returning for the
  "missing return" check).
- `try { ... } catch name { ... }` runs the first block; if any runtime
  error happens inside it, including inside functions it calls, the rest
  of the block is skipped and the `catch` block runs with `name` bound to
  the message. `name` is immutable and visible only in the `catch` block.
- `try` blocks nest; the innermost one catches. A `catch` block may itself
  `fail` to pass the error outward.
- `return`, `break` and `continue` may leave a `try` block normally.
- Not catchable: exceeding `--max-steps`, and invalid bytecode. Everything
  else (including call-depth and size limits) can be caught.

## 13a. Input, output and the system

| Function | Type | Notes |
|---|---|---|
| `input(prompt)` | Str → Str | prints `prompt`, reads one line from standard input (without the newline); runtime error at end of input |
| `read_file(path)` | Str → Str | whole file as UTF-8 |
| `write_file(path, text)` | Str, Str → Unit | creates or replaces |
| `append_file(path, text)` | Str, Str → Unit | creates if missing |
| `file_exists(path)` | Str → Bool | |
| `list_dir(path)` | Str → [Str] | entry names, sorted |
| `delete_file(path)` | Str → Unit | |
| `args()` | → [Str] | arguments after `--` in `nex run file.nex -- a b` |
| `env(name, default)` | Str, Str → Str | environment variable, or `default` |
| `eprint(text)` | Str → Unit | writes a line to standard error |
| `exit(code)` | Int → Unit | ends the program with exit code 0–255 |
| `time_ms()` | → Int | milliseconds since 1970-01-01 UTC |
| `sleep_ms(ms)` | Int → Unit | pauses |

Relative paths are relative to the current working directory. Failed
file operations are runtime errors (catchable) whose message starts with
the function name, e.g. `read_file: cannot read "x.txt": ...`.

`nex run --sandbox` disables the file functions, `env` and all network
functions (they raise `... is disabled (--sandbox)`); the others keep
working.

## 13b. JSON

`Json` is a type for any JSON value. Objects are read with methods; there
is no indexing with `[]`.

```
let j = json_parse("\{\"name\": \"NEX\", \"tags\": [\"fast\"]\}")
say j.get("name").str()          // NEX
say j.get("tags").at(0)          // "fast"   (Json values print as JSON)
let reply = {"ok": json(true), "count": json(3)}
say to_json(reply)               // {"count":3,"ok":true}
```

| Function / method | Type | Notes |
|---|---|---|
| `json_parse(text)` | Str → Json | runtime error on invalid JSON; nesting limit 128 |
| `to_json(value)` | any → Str | compact JSON; maps become objects (non-Str keys become strings), `Float` NaN/inf is an error |
| `json(value)` | any → Json | wrap a value so different types can share one map or array |
| `j.get(key)` | Json | object field, or `null` if missing; error if `j` is not an object |
| `j.has(key)` | Bool | false for non-objects |
| `j.keys()` | [Str] | object keys, sorted |
| `j.at(i)` | Json | array element; error if out of bounds |
| `j.items()` | [Json] | array elements |
| `j.len()` | Int | array, object or string length |
| `j.str()`, `j.int()`, `j.float()`, `j.bool()` | | convert; error if the kind is wrong (`int()` accepts whole numbers) |
| `j.is_null()` | Bool | |
| `j.kind()` | Str | `null`, `bool`, `number`, `string`, `array` or `object` |

Whole JSON numbers that fit in 64 bits are integers; others are floats.
`say` and interpolation show `Json` values as JSON text. In a NEX string
literal, write `\{` and `\}` for JSON braces.

## 13c. HTTP

**Client** (uses the system `curl` program, present on current Windows,
macOS and Linux):

| Function | Type | Notes |
|---|---|---|
| `http_get(url)` | Str → Str | response body |
| `http_post(url, body, content_type)` | Str, Str, Str → Str | response body |

URLs must start with `http://` or `https://`; redirects are followed; the
timeout is 60 seconds. A status of 400 or more, or a network failure, is a
runtime error (catchable).

**Server** (single-threaded, HTTP/1.1, one request at a time):

```
let server = http_listen(8080)
while true {
    let req = http_accept(server)                // waits for a request
    http_respond(req["id"], 200, "text/html", "<h1>Hi {req["path"]}</h1>")
}
```

| Function | Type | Notes |
|---|---|---|
| `http_listen(port)` | Int → Int | listens on `127.0.0.1:port`; `0` picks a free port; returns a server handle |
| `http_port(server)` | Int → Int | the actual port |
| `http_accept(server)` | Int → {Str: Str} | keys `id`, `method`, `path`, `query`, `body`, and `header:<name>` (lower case) |
| `http_respond(id, status, content_type, body)` | Str, Int, Str, Str → Unit | sends the response and closes the connection |

The server only listens on the local machine. To serve the internet, put
it behind a reverse proxy (nginx, Caddy). Request heads are limited to
64 KiB and bodies to 16 MiB; a malformed request gets `400` automatically.

## 13d. TCP and UDP

Sockets are `Int` handles. Data is sent and received as text (received
bytes that are not valid UTF-8 are replaced with `U+FFFD`).

```
let server = tcp_listen("127.0.0.1", 9000)
let conn = tcp_accept(server)
let line = tcp_recv_line(conn)
tcp_send(conn, "you said: {line}\n")
net_close(conn)
```

| Function | Type | Notes |
|---|---|---|
| `tcp_listen(host, port)` | Str, Int → Int | listener; `"0.0.0.0"` listens on all interfaces, port `0` picks a free port |
| `tcp_accept(listener)` | Int → Int | waits for a connection |
| `tcp_connect(host, port)` | Str, Int → Int | 10 second connect timeout per address |
| `tcp_send(conn, text)` | Int, Str → Unit | |
| `tcp_recv(conn, max_bytes)` | Int, Int → Str | waits for data; returns up to `max_bytes`; `""` when the other side closed |
| `tcp_recv_line(conn)` | Int → Str | one line without `\n`; error if the connection is closed |
| `udp_bind(host, port)` | Str, Int → Int | UDP socket |
| `udp_send(sock, host, port, text)` | Int, Str, Int, Str → Unit | |
| `udp_recv(sock, max_bytes)` | Int, Int → {Str: Str} | keys `data`, `host`, `port` |
| `net_timeout(sock, ms)` | Int, Int → Unit | read timeout for receives (`0` = wait forever); a timeout is the error `... timed out` |
| `local_port(sock)` | Int → Int | port of any socket |
| `net_close(sock)` | Int → Unit | |
| `resolve(host)` | Str → [Str] | IP addresses from DNS |

All failures are catchable runtime errors. `--sandbox` blocks all of
these except `net_close`, `net_timeout` and `local_port`.

## 13e. Hashing and encoding

| Function | Type | Notes |
|---|---|---|
| `sha256(text)` | Str → Str | SHA-256 of the UTF-8 bytes, lower-case hex |
| `hmac_sha256(key, text)` | Str, Str → Str | HMAC-SHA-256 (RFC 2104), lower-case hex |
| `hex_encode(text)` | Str → Str | hex of the UTF-8 bytes |
| `base64_encode(text)` | Str → Str | standard alphabet with `=` padding |
| `base64_decode(text)` | Str → Str | error if invalid or not UTF-8 text |

These are implemented in NEX itself and tested against the published
FIPS 180 and RFC 4231 vectors. See `examples/blockchain.nex`.

## 13f. Public-key signatures (Ed25519)

| Function | Type | Notes |
|---|---|---|
| `ed25519_keypair()` | → {Str: Str} | keys `secret` and `public`, 64 hex characters each, from the OS random source |
| `ed25519_public(secret)` | Str → Str | public key for a secret key |
| `ed25519_sign(secret, message)` | Str, Str → Str | 128-hex-character signature (RFC 8032, deterministic) |
| `ed25519_verify(public, message, signature)` | Str, Str, Str → Bool | false for a wrong or malformed signature; error for a malformed public key |
| `random_hex(n)` | Int → Str | `n` (1–1024) secure random bytes as hex |
| `sha512(text)` | Str → Str | SHA-512, hex |

Secret keys are 32-byte seeds in hex, as in RFC 8032. Random bytes come
from the operating system (`RtlGenRandom` on Windows, `/dev/urandom`
elsewhere). The implementation follows TweetNaCl and passes the RFC 8032
test vectors; it has not been independently audited and is not tuned for
speed. See `examples/signatures.nex`.

## 14. Execution model and limits

Programs are compiled to bytecode, verified, then run on a stack VM (see
docs/ARCHITECTURE.md). The VM enforces:

| Limit                         | Default    | CLI flag              |
|-------------------------------|------------|-----------------------|
| Instructions executed         | unlimited  | `--max-steps N`       |
| Call depth                    | 1000       | `--max-call-depth N`  |
| Value stack slots             | 1,000,000  | —                     |
| String length                 | 16 MiB     | —                     |
| Array / map size              | 10,000,000 elements | —            |
| Items in one array/map literal | 30,000    | —                     |
