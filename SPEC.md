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

`app fn let var say return if else while for in break continue true false use`

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
- The expression may not contain a string literal (`"`), and may not span
  lines. Bind the string to a variable first.
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

`use a.b` is parsed and resolved. This version ships **no importable
modules**, so every `use` is a compile error: `unresolved module `a.b``.
Module resolution, circular-import handling, `net.http` and the rest of
the standard library are future milestones (see ROADMAP.md).

## 13. Errors

- **Compile errors** (syntax, names, types) are reported with file, line,
  column and a source snippet. The parser stops at the first syntax error;
  semantic analysis reports every error it finds. A program with any
  compile error never runs.
- **Runtime errors** (overflow, division by zero, index out of bounds,
  missing map key, invalid parse, resource limits) stop the program.
  Output already produced remains. The error shows a call trace with
  source positions. There is no exception handling yet; a runtime error in
  one app stops the remaining apps.

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
