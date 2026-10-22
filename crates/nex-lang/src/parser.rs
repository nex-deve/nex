//! Recursive-descent parser: tokens to AST. Stops at the first syntax error.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::lexer::{lex, StrPart, Tok, Token};
use crate::span::Span;

type PResult<T> = Result<T, Diagnostic>;

pub fn parse(src: &str) -> PResult<Program> {
    parse_file(src, 0, 0).map(|(program, _)| program)
}

/// Parse file number `file`. Expression ids start at `first_id` so that
/// ids stay unique across all files of a project; returns the next free id.
pub fn parse_file(src: &str, file: u32, first_id: ExprId) -> PResult<(Program, ExprId)> {
    let mut parser = Parser {
        tokens: lex_in(src, 0, file)?,
        pos: 0,
        next_id: first_id,
        file,
    };
    let program = parser.program().map_err(|d| in_file(d, file))?;
    Ok((program, parser.next_id))
}

fn in_file(mut d: Diagnostic, file: u32) -> Diagnostic {
    d.span.file = file;
    d
}

fn lex_in(src: &str, base: usize, file: u32) -> PResult<Vec<Token>> {
    let mut tokens = lex(src, base).map_err(|d| in_file(d, file))?;
    for t in &mut tokens {
        t.span.file = file;
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    next_id: ExprId,
    file: u32,
}

fn binop(tok: &Tok) -> Option<(BinOp, u8)> {
    Some(match tok {
        Tok::OrOr => (BinOp::Or, 1),
        Tok::AndAnd => (BinOp::And, 2),
        Tok::EqEq => (BinOp::Eq, 3),
        Tok::NotEq => (BinOp::Ne, 3),
        Tok::Lt => (BinOp::Lt, 4),
        Tok::Le => (BinOp::Le, 4),
        Tok::Gt => (BinOp::Gt, 4),
        Tok::Ge => (BinOp::Ge, 4),
        Tok::Plus => (BinOp::Add, 5),
        Tok::Minus => (BinOp::Sub, 5),
        Tok::Star => (BinOp::Mul, 6),
        Tok::Slash => (BinOp::Div, 6),
        Tok::Percent => (BinOp::Rem, 6),
        _ => return None,
    })
}

/// Turn the left side of `=` into a variable plus index path:
/// `grid[i][j]` becomes (`grid`, [i, j]).
fn into_place(expr: Expr) -> PResult<(Ident, Vec<Expr>)> {
    let mut path = Vec::new();
    let mut cur = expr;
    loop {
        match cur.kind {
            ExprKind::Var(name) => {
                path.reverse();
                return Ok((
                    Ident {
                        name,
                        span: cur.span,
                    },
                    path,
                ));
            }
            ExprKind::Index { base, index } => {
                path.push(*index);
                cur = *base;
            }
            _ => {
                return Err(Diagnostic::error("invalid assignment target", cur.span)
                    .label("cannot assign to this")
                    .note("only a variable or an element like `items[0]` can be assigned"))
            }
        }
    }
}

fn reserved_error(word: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        format!("`{word}` is a reserved keyword and is not supported yet"),
        span,
    )
    .note("see ROADMAP.md for planned language features")
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn prev_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        if token.tok != Tok::Eof {
            self.pos += 1;
        }
        token
    }

    fn check(&self, tok: &Tok) -> bool {
        self.peek() == tok
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.check(tok) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: Tok, what: &str) -> PResult<Span> {
        if self.check(&tok) {
            Ok(self.advance().span)
        } else {
            Err(self.unexpected(what))
        }
    }

    fn unexpected(&self, expected: &str) -> Diagnostic {
        if let Tok::Reserved(word) = self.peek() {
            return reserved_error(word, self.span());
        }
        Diagnostic::error(
            format!("expected {expected}, found {}", self.peek().describe()),
            self.span(),
        )
    }

    /// Skip statement separators (newlines and `;`).
    fn skip_separators(&mut self) {
        while matches!(self.peek(), Tok::Newline | Tok::Semi) {
            self.advance();
        }
    }

    fn skip_newlines(&mut self) {
        while self.check(&Tok::Newline) {
            self.advance();
        }
    }

    fn ident(&mut self, what: &str) -> PResult<Ident> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                let span = self.advance().span;
                Ok(Ident { name, span })
            }
            _ => Err(self.unexpected(what)),
        }
    }

    fn end_of_item(&mut self) -> PResult<()> {
        match self.peek() {
            Tok::Newline | Tok::Semi | Tok::Eof => Ok(()),
            _ => Err(self.unexpected("end of line")),
        }
    }

    fn program(&mut self) -> PResult<Program> {
        let mut items = Vec::new();
        loop {
            self.skip_separators();
            let item = match self.peek() {
                Tok::Eof => break,
                Tok::App => Item::App(self.app()?),
                Tok::Fn => Item::Fn(self.func()?),
                Tok::Use => Item::Use(self.use_decl()?),
                Tok::Let
                | Tok::Var
                | Tok::Say
                | Tok::If
                | Tok::While
                | Tok::Return
                | Tok::Break
                | Tok::Continue
                | Tok::For
                | Tok::Try
                | Tok::Fail => {
                    return Err(Diagnostic::error(
                        "statements are not allowed at the top level",
                        self.span(),
                    )
                    .label("this statement")
                    .note("put statements inside an `app` block or a function"))
                }
                _ => return Err(self.unexpected("`app`, `fn` or `use`")),
            };
            items.push(item);
            self.end_of_item()?;
        }
        Ok(Program { items })
    }

    fn use_decl(&mut self) -> PResult<UseDecl> {
        let start = self.expect(Tok::Use, "`use`")?;
        let mut path = vec![self.ident("module name")?];
        while self.eat(&Tok::Dot) {
            path.push(self.ident("module name")?);
        }
        Ok(UseDecl {
            path,
            span: start.to(self.prev_span()),
        })
    }

    fn app(&mut self) -> PResult<AppDecl> {
        let start = self.expect(Tok::App, "`app`")?;
        let name = self.ident("app name")?;
        let body = self.block()?;
        Ok(AppDecl {
            name,
            span: start.to(body.span),
            body,
        })
    }

    fn func(&mut self) -> PResult<FnDecl> {
        let start = self.expect(Tok::Fn, "`fn`")?;
        let name = self.ident("function name")?;
        self.expect(Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        while !self.check(&Tok::RParen) {
            let pname = self.ident("parameter name or `)`")?;
            if !self.check(&Tok::Colon) {
                return Err(Diagnostic::error(
                    format!("parameter `{}` needs a type annotation", pname.name),
                    pname.span,
                )
                .note(format!("write it as `{}: Int`", pname.name)));
            }
            self.advance();
            let ty = self.type_expr()?;
            params.push(Param { name: pname, ty });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "`,` or `)`")?;
        let ret = if self.eat(&Tok::Arrow) {
            Some(self.type_expr()?)
        } else {
            None
        };
        let body = self.block()?;
        Ok(FnDecl {
            name,
            params,
            ret,
            span: start.to(body.span),
            body,
        })
    }

    fn type_expr(&mut self) -> PResult<TypeExpr> {
        let start = self.span();
        let kind = if self.eat(&Tok::LBracket) {
            let elem = self.type_expr()?;
            self.expect(Tok::RBracket, "`]`")?;
            TypeExprKind::Array(Box::new(elem))
        } else if self.eat(&Tok::LBrace) {
            let key = self.type_expr()?;
            self.expect(Tok::Colon, "`:`")?;
            let value = self.type_expr()?;
            self.expect(Tok::RBrace, "`}`")?;
            TypeExprKind::Map(Box::new(key), Box::new(value))
        } else {
            TypeExprKind::Named(self.ident("type name")?.name)
        };
        Ok(TypeExpr {
            kind,
            span: start.to(self.prev_span()),
        })
    }

    fn block(&mut self) -> PResult<Block> {
        let open = self.expect(Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        loop {
            self.skip_separators();
            if self.check(&Tok::RBrace) {
                break;
            }
            if self.check(&Tok::Eof) {
                return Err(Diagnostic::error("unclosed block: expected `}`", open)
                    .label("this `{` is never closed"));
            }
            stmts.push(self.stmt()?);
            match self.peek() {
                Tok::Newline | Tok::Semi | Tok::RBrace => {}
                _ => return Err(self.unexpected("end of statement (a newline or `;`)")),
            }
        }
        let close = self.advance().span;
        Ok(Block {
            stmts,
            span: open.to(close),
        })
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        let kind = match self.peek().clone() {
            Tok::Let | Tok::Var => {
                let mutable = self.advance().tok == Tok::Var;
                let name = self.ident("variable name")?;
                let ty = if self.eat(&Tok::Colon) {
                    Some(self.type_expr()?)
                } else {
                    None
                };
                self.expect(Tok::Assign, "`=`")?;
                let value = self.expr()?;
                StmtKind::Let {
                    mutable,
                    name,
                    ty,
                    value,
                }
            }
            Tok::Say => {
                self.advance();
                StmtKind::Say(self.expr()?)
            }
            Tok::Return => {
                self.advance();
                if matches!(
                    self.peek(),
                    Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof
                ) {
                    StmtKind::Return(None)
                } else {
                    StmtKind::Return(Some(self.expr()?))
                }
            }
            Tok::If => return self.if_stmt(),
            Tok::While => {
                self.advance();
                let cond = self.expr()?;
                let body = self.block()?;
                StmtKind::While { cond, body }
            }
            Tok::For => {
                self.advance();
                let var = self.ident("loop variable name")?;
                self.expect(Tok::In, "`in`")?;
                let first = self.expr()?;
                let iter = if self.eat(&Tok::DotDot) {
                    ForIter::Range(first, self.expr()?)
                } else {
                    ForIter::Each(first)
                };
                let body = self.block()?;
                StmtKind::For { var, iter, body }
            }
            Tok::Try => {
                self.advance();
                let body = self.block()?;
                let mut look = self.pos;
                while self.tokens[look].tok == Tok::Newline {
                    look += 1;
                }
                if self.tokens[look].tok != Tok::Catch {
                    return Err(self.unexpected("`catch` after the `try` block"));
                }
                self.pos = look;
                self.advance();
                let err = self.ident("a name for the error, like `catch err`")?;
                let handler = self.block()?;
                StmtKind::Try { body, err, handler }
            }
            Tok::Fail => {
                self.advance();
                StmtKind::Fail(self.expr()?)
            }
            Tok::Break => {
                self.advance();
                StmtKind::Break
            }
            Tok::Continue => {
                self.advance();
                StmtKind::Continue
            }
            Tok::LBrace => StmtKind::Block(self.block()?),
            Tok::Fn | Tok::App | Tok::Use => {
                return Err(Diagnostic::error(
                    format!(
                        "`{}` declarations are only allowed at the top level",
                        match self.peek() {
                            Tok::Fn => "fn",
                            Tok::App => "app",
                            _ => "use",
                        }
                    ),
                    self.span(),
                ))
            }
            _ => {
                let expr = self.expr()?;
                if self.check(&Tok::Assign) {
                    self.advance();
                    let (name, path) = into_place(expr)?;
                    let value = self.expr()?;
                    StmtKind::Assign { name, path, value }
                } else {
                    StmtKind::Expr(expr)
                }
            }
        };
        Ok(Stmt {
            kind,
            span: start.to(self.prev_span()),
        })
    }

    fn if_stmt(&mut self) -> PResult<Stmt> {
        let start = self.expect(Tok::If, "`if`")?;
        let cond = self.expr()?;
        let then = self.block()?;
        // `else` may start on the line after the closing `}`.
        let mut look = self.pos;
        while self.tokens[look].tok == Tok::Newline {
            look += 1;
        }
        let else_ = if self.tokens[look].tok == Tok::Else {
            self.pos = look;
            self.advance();
            if self.check(&Tok::If) {
                Some(ElseBranch::If(Box::new(self.if_stmt()?)))
            } else {
                Some(ElseBranch::Block(self.block()?))
            }
        } else {
            None
        };
        Ok(Stmt {
            kind: StmtKind::If { cond, then, else_ },
            span: start.to(self.prev_span()),
        })
    }

    fn mk(&mut self, kind: ExprKind, span: Span) -> Expr {
        let id = self.next_id;
        self.next_id += 1;
        Expr { id, kind, span }
    }

    pub fn expr(&mut self) -> PResult<Expr> {
        self.binary(1)
    }

    fn binary(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, prec)) = binop(self.peek()) {
            if prec < min_prec {
                break;
            }
            let op_span = self.advance().span;
            // An operator at the end of a line continues the expression.
            self.skip_newlines();
            let rhs = self.binary(prec + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = self.mk(
                ExprKind::Binary {
                    op,
                    op_span,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            );
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let op = match self.peek() {
            Tok::Minus => UnOp::Neg,
            Tok::Bang => UnOp::Not,
            _ => return self.postfix(),
        };
        let start = self.advance().span;
        let expr = self.unary()?;
        let span = start.to(expr.span);
        Ok(self.mk(
            ExprKind::Unary {
                op,
                expr: Box::new(expr),
            },
            span,
        ))
    }

    /// A primary expression followed by any `[index]` and `.method(args)`.
    fn postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.primary()?;
        loop {
            if self.eat(&Tok::LBracket) {
                let index = self.expr()?;
                let close = self.expect(Tok::RBracket, "`]`")?;
                let span = expr.span.to(close);
                expr = self.mk(
                    ExprKind::Index {
                        base: Box::new(expr),
                        index: Box::new(index),
                    },
                    span,
                );
            } else if self.eat(&Tok::Dot) {
                let method = self.ident("method name")?;
                self.expect(Tok::LParen, "`(` (methods are called like `items.len()`)")?;
                let args = self.args()?;
                let span = expr.span.to(self.prev_span());
                expr = self.mk(
                    ExprKind::MethodCall {
                        receiver: Box::new(expr),
                        method,
                        args,
                    },
                    span,
                );
            } else {
                return Ok(expr);
            }
        }
    }

    /// Comma-separated arguments after `(`, through the closing `)`.
    fn args(&mut self) -> PResult<Vec<Expr>> {
        let mut args = Vec::new();
        while !self.check(&Tok::RParen) {
            args.push(self.expr()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "`,` or `)`")?;
        Ok(args)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Int(v) => {
                self.advance();
                Ok(self.mk(ExprKind::Int(v), span))
            }
            Tok::Float(v) => {
                self.advance();
                Ok(self.mk(ExprKind::Float(v), span))
            }
            Tok::True | Tok::False => {
                let value = self.advance().tok == Tok::True;
                Ok(self.mk(ExprKind::Bool(value), span))
            }
            Tok::Str(parts) => {
                self.advance();
                self.string(parts, span)
            }
            Tok::Ident(name) => {
                self.advance();
                if !self.check(&Tok::LParen) {
                    return Ok(self.mk(ExprKind::Var(name), span));
                }
                self.advance();
                let args = self.args()?;
                let close = self.prev_span();
                Ok(self.mk(
                    ExprKind::Call {
                        callee: Ident { name, span },
                        args,
                    },
                    span.to(close),
                ))
            }
            Tok::LParen => {
                self.advance();
                let inner = self.expr()?;
                let close = self.expect(Tok::RParen, "`)`")?;
                Ok(Expr {
                    span: span.to(close),
                    ..inner
                })
            }
            Tok::LBracket => {
                self.advance();
                let mut items = Vec::new();
                while !self.check(&Tok::RBracket) {
                    items.push(self.expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                let close = self.expect(Tok::RBracket, "`,` or `]`")?;
                Ok(self.mk(ExprKind::Array(items), span.to(close)))
            }
            Tok::LBrace => {
                self.advance();
                let mut entries = Vec::new();
                self.skip_newlines();
                while !self.check(&Tok::RBrace) {
                    let key = self.expr()?;
                    self.expect(Tok::Colon, "`:` between map key and value")?;
                    self.skip_newlines();
                    let value = self.expr()?;
                    entries.push((key, value));
                    self.skip_newlines();
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                    self.skip_newlines();
                }
                let close = self.expect(Tok::RBrace, "`,` or `}`")?;
                Ok(self.mk(ExprKind::Map(entries), span.to(close)))
            }
            _ => Err(self.unexpected("an expression")),
        }
    }

    fn string(&mut self, parts: Vec<StrPart>, span: Span) -> PResult<Expr> {
        if let [StrPart::Lit(text)] = parts.as_slice() {
            return Ok(self.mk(ExprKind::Str(text.clone()), span));
        }
        let mut out = Vec::new();
        for part in parts {
            match part {
                StrPart::Lit(text) => out.push(InterpPart::Lit(text)),
                StrPart::Expr { src, offset } => {
                    let mut sub = Parser {
                        tokens: lex_in(&src, offset, self.file)?,
                        pos: 0,
                        next_id: self.next_id,
                        file: self.file,
                    };
                    let expr = sub.expr()?;
                    if !sub.check(&Tok::Eof) {
                        return Err(sub.unexpected("`}` to end the interpolation"));
                    }
                    self.next_id = sub.next_id;
                    out.push(InterpPart::Expr(expr));
                }
            }
        }
        Ok(self.mk(ExprKind::Interp(out), span))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_app_stmts(src: &str) -> Vec<Stmt> {
        let program = parse(src).unwrap();
        match &program.items[0] {
            Item::App(app) => app.body.stmts.clone(),
            other => panic!("expected app, got {other:?}"),
        }
    }

    fn say_expr(src: &str) -> Expr {
        let stmts = first_app_stmts(&format!("app A {{ say {src} }}"));
        match &stmts[0].kind {
            StmtKind::Say(e) => e.clone(),
            other => panic!("expected say, got {other:?}"),
        }
    }

    fn shape(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(v) => v.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(s) => format!("{s:?}"),
            ExprKind::Var(n) => n.clone(),
            ExprKind::Unary { op, expr } => format!("({op:?} {})", shape(expr)),
            ExprKind::Binary { op, lhs, rhs, .. } => {
                format!("({} {} {})", shape(lhs), op.symbol(), shape(rhs))
            }
            ExprKind::Call { callee, args } => format!(
                "{}({})",
                callee.name,
                args.iter().map(shape).collect::<Vec<_>>().join(", ")
            ),
            ExprKind::Interp(parts) => parts
                .iter()
                .map(|p| match p {
                    InterpPart::Lit(s) => format!("{s:?}"),
                    InterpPart::Expr(e) => format!("{{{}}}", shape(e)),
                })
                .collect::<Vec<_>>()
                .join(""),
            ExprKind::Float(v) => format!("{v:?}"),
            ExprKind::Array(items) => format!(
                "[{}]",
                items.iter().map(shape).collect::<Vec<_>>().join(", ")
            ),
            ExprKind::Map(entries) => format!(
                "{{{}}}",
                entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", shape(k), shape(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ExprKind::Index { base, index } => format!("{}[{}]", shape(base), shape(index)),
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => format!(
                "{}.{}({})",
                shape(receiver),
                method.name,
                args.iter().map(shape).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    #[test]
    fn collections_and_postfix() {
        assert_eq!(shape(&say_expr("[1, 2.5, x]")), "[1, 2.5, x]");
        assert_eq!(shape(&say_expr("[]")), "[]");
        assert_eq!(
            shape(&say_expr("{\"a\": 1,\n \"b\": 2,\n}")),
            "{\"a\": 1, \"b\": 2}"
        );
        assert_eq!(shape(&say_expr("{}")), "{}");
        assert_eq!(shape(&say_expr("grid[i][j] + 1")), "(grid[i][j] + 1)");
        assert_eq!(
            shape(&say_expr("-xs.slice(0, 2).len()")),
            "(Neg xs.slice(0, 2).len())"
        );
        assert_eq!(shape(&say_expr("m[\"k\"].upper()")), "m[\"k\"].upper()");
    }

    #[test]
    fn try_catch_and_fail() {
        let stmts =
            first_app_stmts("app A {\n try {\n fail \"x\"\n }\n catch err {\n say err\n }\n}");
        match &stmts[0].kind {
            StmtKind::Try { body, err, handler } => {
                assert!(matches!(body.stmts[0].kind, StmtKind::Fail(_)));
                assert_eq!(err.name, "err");
                assert_eq!(handler.stmts.len(), 1);
            }
            other => panic!("{other:?}"),
        }
        assert!(parse("app A { try { } }")
            .unwrap_err()
            .message
            .contains("`catch` after the `try` block"));
        assert!(parse("app A { try { } catch { } }")
            .unwrap_err()
            .message
            .contains("a name for the error"));
    }

    #[test]
    fn spans_carry_the_file_number() {
        let (p, next) = parse_file("app A { say 1 }", 3, 100).unwrap();
        let Item::App(a) = &p.items[0] else { panic!() };
        assert_eq!(a.name.span.file, 3);
        let StmtKind::Say(e) = &a.body.stmts[0].kind else {
            panic!()
        };
        assert_eq!(e.id, 100);
        assert_eq!(e.span.file, 3);
        assert_eq!(next, 101);
        let err = parse_file("app A { say @ }", 2, 0).unwrap_err();
        assert_eq!(err.span.file, 2);
    }

    #[test]
    fn assignment_targets_and_for() {
        let stmts =
            first_app_stmts("app A {\n grid[1][2] = 5\n for i in 0..n { }\n for x in items { }\n}");
        match &stmts[0].kind {
            StmtKind::Assign { name, path, .. } => {
                assert_eq!(name.name, "grid");
                assert_eq!(path.len(), 2);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            &stmts[1].kind,
            StmtKind::For {
                iter: ForIter::Range(..),
                ..
            }
        ));
        assert!(matches!(
            &stmts[2].kind,
            StmtKind::For {
                iter: ForIter::Each(_),
                ..
            }
        ));
        let p = parse("fn f(a: [Int], m: {Str: [Bool]}) {}").unwrap();
        let Item::Fn(f) = &p.items[0] else { panic!() };
        assert!(matches!(f.params[0].ty.kind, TypeExprKind::Array(_)));
        assert!(matches!(f.params[1].ty.kind, TypeExprKind::Map(..)));
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(shape(&say_expr("1 + 2 * 3")), "(1 + (2 * 3))");
        assert_eq!(shape(&say_expr("10 - 3 - 2")), "((10 - 3) - 2)");
        assert_eq!(shape(&say_expr("(1 + 2) * 3")), "((1 + 2) * 3)");
        assert_eq!(
            shape(&say_expr("a < b && b < c || !d")),
            "(((a < b) && (b < c)) || (Not d))"
        );
        assert_eq!(shape(&say_expr("-x == 1")), "((Neg x) == 1)");
        assert_eq!(shape(&say_expr("f(1, g(2),)")), "f(1, g(2))");
    }

    #[test]
    fn operator_at_line_end_continues_expression() {
        assert_eq!(shape(&say_expr("1 +\n 2")), "(1 + 2)");
    }

    #[test]
    fn interpolation_parses_inner_expressions() {
        let e = say_expr(r#""a {x + 1} b""#);
        assert_eq!(shape(&e), r#""a "{(x + 1)}" b""#);
        if let ExprKind::Interp(parts) = &e.kind {
            if let InterpPart::Expr(inner) = &parts[1] {
                // `x` sits at byte 16 of `app A { say "a {x + 1} b" }`.
                assert_eq!(inner.span.start, 16);
            }
        }
    }

    #[test]
    fn statements() {
        let stmts = first_app_stmts(
            "app A {\n let x = 1\n var y: Int = 2; y = 3\n if x == 1 { say 1 }\n else if x == 2 { say 2 } else { say 3 }\n while false { break }\n { continue }\n f()\n return\n}",
        );
        let kinds: Vec<&str> = stmts
            .iter()
            .map(|s| match &s.kind {
                StmtKind::Let { mutable: false, .. } => "let",
                StmtKind::Let { mutable: true, .. } => "var",
                StmtKind::Assign { .. } => "assign",
                StmtKind::Say(_) => "say",
                StmtKind::Return(_) => "return",
                StmtKind::If { .. } => "if",
                StmtKind::While { .. } => "while",
                StmtKind::For { .. } => "for",
                StmtKind::Try { .. } => "try",
                StmtKind::Fail(_) => "fail",
                StmtKind::Break => "break",
                StmtKind::Continue => "continue",
                StmtKind::Block(_) => "block",
                StmtKind::Expr(_) => "expr",
            })
            .collect();
        assert_eq!(
            kinds,
            ["let", "var", "assign", "if", "while", "block", "expr", "return"]
        );
    }

    #[test]
    fn functions_and_uses() {
        let p = parse("use net.http\nfn add(a: Int, b: Int) -> Int { return a + b }\nfn hi() { }")
            .unwrap();
        assert_eq!(p.items.len(), 3);
        match &p.items[0] {
            Item::Use(u) => assert_eq!(u.path.len(), 2),
            _ => panic!(),
        }
        match &p.items[1] {
            Item::Fn(f) => {
                assert_eq!(f.params.len(), 2);
                assert_eq!(
                    f.ret.as_ref().unwrap().kind,
                    TypeExprKind::Named("Int".into())
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn syntax_errors() {
        let cases = [
            ("say 1", "not allowed at the top level"),
            ("app A { let = 1 }", "expected variable name"),
            ("app A { say 1 say 2 }", "expected end of statement"),
            ("app A { say (1 + 2 }", "expected `)`"),
            ("app A {\n say 1\n", "unclosed block"),
            ("fn f(a) {}", "needs a type annotation"),
            ("app A { match x }", "reserved keyword"),
            ("app A { for x }", "expected `in`"),
            ("app A { say [1, }", "expected an expression"),
            (
                "app A { say {1 2} }",
                "expected `:` between map key and value",
            ),
            ("app A { f() = 1 }", "invalid assignment target"),
            ("app A { say x. }", "expected method name"),
            ("app A { say x.len }", "methods are called like"),
            ("app A { fn g() {} }", "only allowed at the top level"),
            ("app A { say \"{1 +}\" }", "expected an expression"),
            ("app A { say \"{1 2}\" }", "to end the interpolation"),
            ("x", "expected `app`, `fn` or `use`"),
        ];
        for (src, want) in cases {
            let err = parse(src).expect_err(src);
            assert!(
                err.message.contains(want),
                "{src:?}: got {:?}, want {want:?}",
                err.message
            );
        }
    }
}
