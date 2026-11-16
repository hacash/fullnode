use super::*;

pub(super) struct Cursor {
    tokens: Vec<Token>,
    idx: usize,
    /// P2-D03 T1 sidecar, parallel to `tokens` (may be shorter/empty when
    /// built through the spanless path). Synthesized tokens carry
    /// `TokenSpan::NONE`.
    spans: Vec<TokenSpan>,
}

impl Cursor {
    pub(super) fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, idx: 0, spans: Vec::new() }
    }

    pub(super) fn with_spans(tokens: Vec<Token>, spans: Vec<TokenSpan>) -> Self {
        Self { tokens, idx: 0, spans }
    }

    pub(super) fn at_end(&self) -> bool {
        self.idx >= self.tokens.len()
    }

    pub(super) fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.idx)
    }

    /// Span of the current token (with last-known fallback for EOF);
    /// `None` when no span information exists (spanless path).
    pub(super) fn span(&self) -> Option<TokenSpan> {
        if self.idx < self.spans.len() && self.spans[self.idx].known() {
            return Some(self.spans[self.idx]);
        }
        self.spans.iter().rev().find(|s| s.known()).copied()
    }

    /// `{line}:{col}: ` prefix for error messages (empty without spans).
    pub(super) fn pos_prefix(&self) -> String {
        self.span().map(|s| format!("{s}: ")).unwrap_or_default()
    }

    pub(super) fn next(&mut self) -> Ret<Token> {
        if self.idx >= self.tokens.len() {
            let pre = self.pos_prefix();
            return Err(Error::fault(format!(
                "{pre}syntax: unexpected end of token stream"
            )));
        }
        let token = self.tokens[self.idx].clone();
        self.idx += 1;
        Ok(token)
    }

    pub(super) fn rewind_one(&mut self) {
        if self.idx > 0 {
            self.idx -= 1;
        }
    }

    pub(super) fn eat_keyword(&mut self, kw: KwTy) -> bool {
        if matches!(self.peek(), Some(Token::Keyword(got)) if *got == kw) {
            self.idx += 1;
            return true;
        }
        false
    }

    pub(super) fn eat_partition(&mut self, ch: char) -> bool {
        if matches!(self.peek(), Some(Token::Partition(got)) if *got == ch) {
            self.idx += 1;
            return true;
        }
        false
    }

    pub(super) fn skip_soft_separators(&mut self) {
        while self.eat_partition(',') {}
    }

    pub(super) fn expect_keyword(&mut self, kw: KwTy, err_msg: &'static str) -> Ret<()> {
        let token = self.next()?;
        let Token::Keyword(got) = token else {
            return errf!("{}", err_msg);
        };
        if got != kw {
            return errf!("{}", err_msg);
        }
        Ok(())
    }

    pub(super) fn expect_partition(&mut self, ch: char, err_msg: &'static str) -> Ret<()> {
        let token = self.next()?;
        let Token::Partition(got) = token else {
            return errf!("{}", err_msg);
        };
        if got != ch {
            return errf!("{}", err_msg);
        }
        Ok(())
    }
}
