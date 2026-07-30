use super::*;

pub(super) struct Cursor {
    tokens: Vec<Token>,
    idx: usize,
}

impl Cursor {
    pub(super) fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, idx: 0 }
    }

    pub(super) fn at_end(&self) -> bool {
        self.idx >= self.tokens.len()
    }

    pub(super) fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.idx)
    }

    pub(super) fn next(&mut self) -> Ret<Token> {
        if self.idx >= self.tokens.len() {
            return errf!("syntax: unexpected end of token stream");
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
