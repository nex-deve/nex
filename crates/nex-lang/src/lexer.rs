//! Lexer: source text to tokens.
//!
//! Newlines are significant statement terminators, except inside `(...)`
//! and `[...]`. Consecutive newlines collapse into one token.

use crate::diag::Diagnostic;
use crate::span::Span;

/// A piece of a string literal: literal text or an interpolated expression.
#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    Lit(String),
    /// Raw source of an `{...}` interpolation and its absolute byte offset.
    Expr {
        src: String,
        offset: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(Vec<StrPart>),
    /// A word reserved for future versions of the language.
    Reserved(String),
    // keywords
    App,
    Fn,
    Let,
    Var,
    Say,
    Return,
    If,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    True,
    False,
    Use,
    Try,
    Catch,
    Fail,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Semi,
    Dot,
    DotDot,
    Arrow,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Newline,
    Eof,
}

/// Words that are not identifiers because a future version may use them.
pub const RESERVED: &[&str] = &[
    "const", "struct", "enum", "match", "loop", "pub", "mod", "import", "nil",
];

impl Tok {
    /// Human-readable description used in "expected X, found Y" errors.
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(name) => format!("identifier `{name}`"),
            Tok::Int(v) => format!("integer `{v}`"),
            Tok::Float(v) => format!("number `{v}`"),
            Tok::Str(_) => "string literal".to_string(),
            Tok::Reserved(word) => format!("reserved keyword `{word}`"),
            Tok::Newline => "end of line".to_string(),
            Tok::Eof => "end of file".to_string(),
            other => format!("`{}`", other.text()),
        }
    }

    fn text(&self) -> &'static str {
        match self {
            Tok::App => "app",
            Tok::Fn => "fn",
            Tok::Let => "let",
            Tok::Var => "var",
            Tok::Say => "say",
            Tok::Return => "return",
            Tok::If => "if",
            Tok::Else => "else",
            Tok::While => "while",
            Tok::For => "for",
            Tok::In => "in",
            Tok::Break => "break",
            Tok::Continue => "continue",
            Tok::True => "true",
            Tok::False => "false",
            Tok::Use => "use",
            Tok::Try => "try",
            Tok::Catch => "catch",
            Tok::Fail => "fail",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::Semi => ";",
            Tok::Dot => ".",
            Tok::DotDot => "..",
            Tok::Arrow => "->",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Assign => "=",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Lt => "<",
            Tok::Le => "<=",
            Tok::Gt => ">",
            Tok::Ge => ">=",
            Tok::AndAnd => "&&",
            Tok::OrOr => "||",
            Tok::Bang => "!",
            Tok::Ident(_)
            | Tok::Int(_)
            | Tok::Float(_)
            | Tok::Str(_)
            | Tok::Reserved(_)
            | Tok::Newline
            | Tok::Eof => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// Tokenize `src`. `base` is added to every span so that interpolated
/// expressions keep their position in the enclosing file.
pub fn lex(src: &str, base: usize) -> Result<Vec<Token>, Diagnostic> {
    Lexer {
        src,
        pos: 0,
        base,
        depth: 0,
        tokens: Vec::new(),
    }
    .run()
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    base: usize,
    /// Nesting depth of `(` and `[`; newlines inside are ignored.
    depth: usize,
    tokens: Vec<Token>,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek2(&self) -> Option<char> {
        let mut chars = self.src[self.pos..].chars();
        chars.next();
        chars.next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn span(&self, start: usize) -> Span {
        Span::new(self.base + start, self.base + self.pos)
    }

    fn push(&mut self, tok: Tok, start: usize) {
        let span = self.span(start);
        self.tokens.push(Token { tok, span });
    }

    fn run(mut self) -> Result<Vec<Token>, Diagnostic> {
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '\n' => {
                    self.bump();
                    let after_newline = matches!(
                        self.tokens.last().map(|t| &t.tok),
                        None | Some(Tok::Newline)
                    );
                    if self.depth == 0 && !after_newline {
                        self.push(Tok::Newline, start);
                    }
                }
                '/' if self.peek2() == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                '/' if self.peek2() == Some('*') => self.block_comment()?,
                '"' => self.string()?,
                '0'..='9' => self.number()?,
                c if c.is_ascii_alphabetic() || c == '_' => self.ident(),
                _ => self.punct()?,
            }
        }
        let end = self.pos;
        self.push(Tok::Eof, end);
        Ok(self.tokens)
    }

    fn block_comment(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        self.bump();
        self.bump();
        loop {
            match self.peek() {
                None => {
                    return Err(Diagnostic::error(
                        "unterminated block comment",
                        Span::new(self.base + start, self.base + start + 2),
                    )
                    .label("comment starts here"))
                }
                Some('*') if self.peek2() == Some('/') => {
                    self.bump();
                    self.bump();
                    return Ok(());
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    fn ident(&mut self) {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                self.bump();
            } else {
                break;
            }
        }
        let text = &self.src[start..self.pos];
        let tok = match text {
            "app" => Tok::App,
            "fn" => Tok::Fn,
            "let" => Tok::Let,
            "var" => Tok::Var,
            "say" => Tok::Say,
            "return" => Tok::Return,
            "if" => Tok::If,
            "else" => Tok::Else,
            "while" => Tok::While,
            "for" => Tok::For,
            "in" => Tok::In,
            "break" => Tok::Break,
            "continue" => Tok::Continue,
            "true" => Tok::True,
            "false" => Tok::False,
            "use" => Tok::Use,
            "try" => Tok::Try,
            "catch" => Tok::Catch,
            "fail" => Tok::Fail,
            _ if RESERVED.contains(&text) => Tok::Reserved(text.to_string()),
            _ => Tok::Ident(text.to_string()),
        };
        self.push(tok, start);
    }

    fn number(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        let mut digits = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                digits.push(c);
                self.bump();
            } else if c == '_' {
                self.bump();
            } else {
                break;
            }
        }
        let mut is_float = false;
        // `1.5` is a float; `1..5` is a range and `1.len` is not a number.
        if self.peek() == Some('.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            digits.push('.');
            self.bump();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    digits.push(c);
                    self.bump();
                } else if c == '_' {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let rest = &self.src[self.pos + 1..];
            let sign = usize::from(rest.starts_with(['+', '-']));
            if rest[sign..].starts_with(|c: char| c.is_ascii_digit()) {
                is_float = true;
                digits.push('e');
                self.bump();
                if sign == 1 {
                    digits.push(self.bump().expect("sign"));
                }
                while let Some(c) = self.peek().filter(|c| c.is_ascii_digit()) {
                    digits.push(c);
                    self.bump();
                }
            }
        }
        if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            while self.peek().is_some_and(|c| c.is_ascii_alphanumeric()) {
                self.bump();
            }
            return Err(
                Diagnostic::error("invalid number literal", self.span(start))
                    .note("identifiers cannot start with a digit"),
            );
        }
        if is_float {
            let value: f64 = digits.parse().map_err(|_| {
                Diagnostic::error("invalid floating-point literal", self.span(start))
            })?;
            if !value.is_finite() {
                return Err(Diagnostic::error(
                    "floating-point literal is too large",
                    self.span(start),
                ));
            }
            self.push(Tok::Float(value), start);
            return Ok(());
        }
        let value: i64 = digits.parse().map_err(|_| {
            Diagnostic::error("integer literal is too large for `Int`", self.span(start))
                .note("`Int` is a 64-bit signed integer (max 9223372036854775807)")
        })?;
        self.push(Tok::Int(value), start);
        Ok(())
    }

    fn string(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        let quote = Span::new(self.base + start, self.base + start + 1);
        self.bump();
        let mut parts = Vec::new();
        let mut lit = String::new();
        loop {
            let c_start = self.pos;
            match self.bump() {
                None | Some('\n') => {
                    return Err(Diagnostic::error("unterminated string literal", quote)
                        .label("string starts here")
                        .note("strings must close with `\"` on the same line"))
                }
                Some('"') => break,
                Some('\\') => {
                    let ch = match self.bump() {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('0') => '\0',
                        Some('\\') => '\\',
                        Some('"') => '"',
                        Some('{') => '{',
                        Some('}') => '}',
                        None | Some('\n') => {
                            return Err(Diagnostic::error("unterminated string literal", quote)
                                .label("string starts here"))
                        }
                        Some(other) => {
                            return Err(Diagnostic::error(
                                format!("unknown escape sequence `\\{other}`"),
                                self.span(c_start),
                            )
                            .note("valid escapes are \\n \\t \\r \\0 \\\\ \\\" \\{ \\}"))
                        }
                    };
                    lit.push(ch);
                }
                Some('{') => {
                    let open = Span::new(self.base + c_start, self.base + c_start + 1);
                    let expr_start = self.pos;
                    if !self.skip_interpolation() {
                        return Err(Diagnostic::error(
                            "unterminated `{` interpolation in string literal",
                            open,
                        )
                        .note("write `\\{` for a literal `{`"));
                    }
                    let expr_src = &self.src[expr_start..self.pos];
                    self.bump();
                    if expr_src.trim().is_empty() {
                        return Err(Diagnostic::error(
                            "empty `{}` interpolation",
                            Span::new(self.base + c_start, self.base + self.pos),
                        )
                        .note("write `\\{` for a literal `{`"));
                    }
                    if !lit.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut lit)));
                    }
                    parts.push(StrPart::Expr {
                        src: expr_src.to_string(),
                        offset: self.base + expr_start,
                    });
                }
                Some(c) => lit.push(c),
            }
        }
        if !lit.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(lit));
        }
        self.push(Tok::Str(parts), start);
        Ok(())
    }

    /// Move to the `}` that closes an interpolation whose `{` was just
    /// consumed. String literals inside it are skipped as a whole (so
    /// `"{f("}")}"` works), including their own interpolations. Returns
    /// false at a newline or end of input.
    fn skip_interpolation(&mut self) -> bool {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                None | Some('\n') => return false,
                Some('}') if depth == 0 => return true,
                Some('}') => depth -= 1,
                Some('{') => depth += 1,
                Some('"') => {
                    self.bump();
                    if !self.skip_string_body() {
                        return false;
                    }
                    continue;
                }
                Some(_) => {}
            }
            self.bump();
        }
    }

    /// Skip a string literal whose opening `"` was just consumed, through
    /// its closing `"`.
    fn skip_string_body(&mut self) -> bool {
        loop {
            match self.bump() {
                None | Some('\n') => return false,
                Some('"') => return true,
                Some('\\') => {
                    if matches!(self.bump(), None | Some('\n')) {
                        return false;
                    }
                }
                Some('{') => {
                    if !self.skip_interpolation() {
                        return false;
                    }
                    self.bump();
                }
                Some(_) => {}
            }
        }
    }

    fn punct(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        let c = self.bump().expect("punct called at end of input");
        let next = self.peek();
        let two = |tok: Tok, lexer: &mut Self| {
            lexer.bump();
            tok
        };
        let tok = match (c, next) {
            ('-', Some('>')) => two(Tok::Arrow, self),
            ('=', Some('=')) => two(Tok::EqEq, self),
            ('!', Some('=')) => two(Tok::NotEq, self),
            ('<', Some('=')) => two(Tok::Le, self),
            ('>', Some('=')) => two(Tok::Ge, self),
            ('&', Some('&')) => two(Tok::AndAnd, self),
            ('|', Some('|')) => two(Tok::OrOr, self),
            ('(', _) => {
                self.depth += 1;
                Tok::LParen
            }
            (')', _) => {
                self.depth = self.depth.saturating_sub(1);
                Tok::RParen
            }
            ('[', _) => {
                self.depth += 1;
                Tok::LBracket
            }
            (']', _) => {
                self.depth = self.depth.saturating_sub(1);
                Tok::RBracket
            }
            ('{', _) => Tok::LBrace,
            ('}', _) => Tok::RBrace,
            (',', _) => Tok::Comma,
            (':', _) => Tok::Colon,
            (';', _) => Tok::Semi,
            ('.', Some('.')) => two(Tok::DotDot, self),
            ('.', _) => Tok::Dot,
            ('+', _) => Tok::Plus,
            ('-', _) => Tok::Minus,
            ('*', _) => Tok::Star,
            ('/', _) => Tok::Slash,
            ('%', _) => Tok::Percent,
            ('=', _) => Tok::Assign,
            ('<', _) => Tok::Lt,
            ('>', _) => Tok::Gt,
            ('!', _) => Tok::Bang,
            _ => {
                let mut d = Diagnostic::error(
                    format!("unexpected character `{}`", c.escape_debug()),
                    self.span(start),
                );
                if c.is_alphabetic() {
                    d = d.note("identifiers may only contain ASCII letters, digits and `_`");
                }
                return Err(d);
            }
        };
        self.push(tok, start);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        lex(src, 0).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn keywords_idents_and_numbers() {
        assert_eq!(
            toks("let x = 1_000"),
            vec![
                Tok::Let,
                Tok::Ident("x".into()),
                Tok::Assign,
                Tok::Int(1000),
                Tok::Eof
            ]
        );
        assert_eq!(toks("match"), vec![Tok::Reserved("match".into()), Tok::Eof]);
        assert_eq!(toks("for in"), vec![Tok::For, Tok::In, Tok::Eof]);
    }

    #[test]
    fn floats_and_ranges() {
        assert_eq!(
            toks("1.5 2e3 1_000.25 3.0E-2"),
            vec![
                Tok::Float(1.5),
                Tok::Float(2000.0),
                Tok::Float(1000.25),
                Tok::Float(0.03),
                Tok::Eof
            ]
        );
        assert_eq!(
            toks("0..10"),
            vec![Tok::Int(0), Tok::DotDot, Tok::Int(10), Tok::Eof]
        );
        assert_eq!(
            toks("x.len"),
            vec![
                Tok::Ident("x".into()),
                Tok::Dot,
                Tok::Ident("len".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn newlines_collapse_and_are_ignored_in_parens() {
        assert_eq!(
            toks("a\n\n\nb(\n1,\n2)\n"),
            vec![
                Tok::Ident("a".into()),
                Tok::Newline,
                Tok::Ident("b".into()),
                Tok::LParen,
                Tok::Int(1),
                Tok::Comma,
                Tok::Int(2),
                Tok::RParen,
                Tok::Newline,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn operators() {
        assert_eq!(
            toks("-> == != <= >= && || ! < > = %"),
            vec![
                Tok::Arrow,
                Tok::EqEq,
                Tok::NotEq,
                Tok::Le,
                Tok::Ge,
                Tok::AndAnd,
                Tok::OrOr,
                Tok::Bang,
                Tok::Lt,
                Tok::Gt,
                Tok::Assign,
                Tok::Percent,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            toks("a // hi\n/* multi\nline */ b"),
            vec![
                Tok::Ident("a".into()),
                Tok::Newline,
                Tok::Ident("b".into()),
                Tok::Eof
            ]
        );
        assert!(lex("/* open", 0).is_err());
    }

    #[test]
    fn string_escapes_and_interpolation() {
        let t = lex(r#""a\n\{b\} {x + 1}!""#, 0).unwrap();
        assert_eq!(
            t[0].tok,
            Tok::Str(vec![
                StrPart::Lit("a\n{b} ".into()),
                StrPart::Expr {
                    src: "x + 1".into(),
                    offset: 11
                },
                StrPart::Lit("!".into()),
            ])
        );
        assert_eq!(
            toks(r#""""#)[0],
            Tok::Str(vec![StrPart::Lit(String::new())])
        );
    }

    #[test]
    fn string_errors() {
        assert!(lex("\"abc", 0)
            .unwrap_err()
            .message
            .contains("unterminated"));
        assert!(lex("\"a\nb\"", 0).is_err());
        assert!(lex(r#""\q""#, 0)
            .unwrap_err()
            .message
            .contains("unknown escape"));
        assert!(lex(r#""{}""#, 0).unwrap_err().message.contains("empty"));
        assert!(lex(r#""{x""#, 0).is_err());
        assert!(lex(r#""{f("x)}""#, 0).is_err());
    }

    #[test]
    fn strings_inside_interpolation() {
        // A string literal inside `{...}` may contain braces and quotes.
        let t = lex(r#""a {f("}", "\"")} b {g("x {y} z")}""#, 0).unwrap();
        let Tok::Str(parts) = &t[0].tok else { panic!() };
        let exprs: Vec<&str> = parts
            .iter()
            .filter_map(|p| match p {
                StrPart::Expr { src, .. } => Some(src.as_str()),
                StrPart::Lit(_) => None,
            })
            .collect();
        assert_eq!(exprs, [r#"f("}", "\"")"#, r#"g("x {y} z")"#]);
    }

    #[test]
    fn number_errors() {
        assert!(lex("99999999999999999999", 0)
            .unwrap_err()
            .message
            .contains("too large"));
        assert!(lex("1e999", 0).unwrap_err().message.contains("too large"));
        assert!(lex("12abc", 0).is_err());
        assert!(lex("1.5x", 0).is_err());
        assert_eq!(toks("9223372036854775807")[0], Tok::Int(i64::MAX));
    }

    #[test]
    fn unexpected_character_has_span() {
        let err = lex("a @", 0).unwrap_err();
        assert_eq!(err.span, Span::new(2, 3));
        assert!(lex("é", 0).unwrap_err().notes[0].contains("ASCII"));
    }

    #[test]
    fn unicode_in_strings() {
        assert_eq!(
            toks("\"হ্যালো 👋\"")[0],
            Tok::Str(vec![StrPart::Lit("হ্যালো 👋".into())])
        );
    }
}
