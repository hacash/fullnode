/// Dedicated assembler lexer. FitSH `Tokenizer` is not used: intro names collide
/// with `KwTy` (`end`/`return`/`call`/`assert`/`print`/`abort`).

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsmTok {
    Ident(String),
    Integer(u128),
    Hex(Vec<u8>),
    Label(String),
    Op(String),
    Comma,
    Colon,
    LBrace,
    RBrace,
    LParen,
    RParen,
}

#[derive(Clone, Debug)]
pub struct SpannedTok {
    pub tok: AsmTok,
    pub start: usize,
    pub end: usize,
}

pub fn lex_asm(text: &str) -> Ret<Vec<SpannedTok>> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();
    while i < bytes.len() {
        let c = bytes[i];
        if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' {
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i = skip_to_eol(bytes, i);
            continue;
        }
        if c == b';' {
            i = skip_to_eol(bytes, i);
            continue;
        }
        let start = i;
        let tok = if c == b'.' {
            i += 1;
            let ident = take_ident(bytes, &mut i);
            if ident.is_empty() {
                return errf!("assembler: lone '.' at {}", start);
            }
            AsmTok::Label(ident)
        } else if c == b'0'
            && i + 1 < bytes.len()
            && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
        {
            i += 2;
            let hex_start = i;
            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            let hex = &text[hex_start..i];
            if hex.is_empty() {
                return errf!("assembler: empty hex literal at {}", start);
            }
            if hex.len() % 2 != 0 {
                return errf!("assembler: odd-length hex literal 0x{} at {}", hex, start);
            }
            let data = hex::decode(hex).map_err(|e| Error::normal(e.to_string()))?;
            AsmTok::Hex(data)
        } else if c.is_ascii_digit() {
            let n = take_integer(bytes, &mut i, start)?;
            AsmTok::Integer(n)
        } else if is_ident_start(c) {
            let ident = take_ident(bytes, &mut i);
            AsmTok::Ident(ident)
        } else if let Some(op) = take_op(bytes, &mut i) {
            AsmTok::Op(op)
        } else {
            i += 1;
            match c {
                b',' => AsmTok::Comma,
                b':' => AsmTok::Colon,
                b'{' => AsmTok::LBrace,
                b'}' => AsmTok::RBrace,
                b'(' => AsmTok::LParen,
                b')' => AsmTok::RParen,
                _ => {
                    return errf!(
                        "assembler: unexpected character {:?} at {}",
                        c as char,
                        start
                    )
                }
            }
        };
        out.push(SpannedTok {
            tok,
            start,
            end: i,
        });
    }
    Ok(out)
}

fn skip_to_eol(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

fn is_ident_cont(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

fn take_ident(bytes: &[u8], i: &mut usize) -> String {
    let start = *i;
    if *i < bytes.len() && is_ident_start(bytes[*i]) {
        *i += 1;
        while *i < bytes.len() && is_ident_cont(bytes[*i]) {
            *i += 1;
        }
    }
    String::from_utf8_lossy(&bytes[start..*i]).into_owned()
}

fn take_integer(bytes: &[u8], i: &mut usize, start: usize) -> Ret<u128> {
    let begin = *i;
    while *i < bytes.len() && bytes[*i].is_ascii_digit() {
        *i += 1;
    }
    let s = std::str::from_utf8(&bytes[begin..*i]).unwrap_or("");
    s.parse::<u128>()
        .map_err(|_| Error::normal(format!("assembler: integer overflow at {}", start)))
}

fn take_op(bytes: &[u8], i: &mut usize) -> Option<String> {
    let rest = &bytes[*i..];
    let two = if rest.len() >= 2 {
        Some(&rest[..2])
    } else {
        None
    };
    let matched = match two {
        Some(b"+=") | Some(b"-=") | Some(b"*=") | Some(b"/=") | Some(b"&&") | Some(b"||")
        | Some(b"==") | Some(b"!=") | Some(b">=") | Some(b"<=") => 2,
        _ => match rest.first() {
            Some(b'>') | Some(b'<') => 1,
            _ => return None,
        },
    };
    let s = std::str::from_utf8(&bytes[*i..*i + matched])
        .unwrap_or("")
        .to_string();
    *i += matched;
    Some(s)
}

#[derive(Clone)]
pub struct TokCursor<'a> {
    src: &'a str,
    toks: &'a [SpannedTok],
    pos: usize,
}

impl<'a> TokCursor<'a> {
    pub fn new(src: &'a str, toks: &'a [SpannedTok]) -> Self {
        Self { src, toks, pos: 0 }
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.toks.len()
    }

    pub fn peek(&self) -> Option<&AsmTok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    pub fn next(&mut self) -> Option<&SpannedTok> {
        let t = self.toks.get(self.pos)?;
        self.pos += 1;
        Some(t)
    }

    pub fn skip_commas(&mut self) {
        while matches!(self.peek(), Some(AsmTok::Comma)) {
            self.pos += 1;
        }
    }

    pub fn expect_lbrace(&mut self) -> Ret<()> {
        match self.next().map(|t| &t.tok) {
            Some(AsmTok::LBrace) => Ok(()),
            other => errf!("assembler: expected '{{', got {:?}", other),
        }
    }

    pub fn expect_rbrace(&mut self) -> Ret<()> {
        match self.next().map(|t| &t.tok) {
            Some(AsmTok::RBrace) => Ok(()),
            other => errf!("assembler: expected '}}', got {:?}", other),
        }
    }

    pub fn brace_inner_src(&self) -> Ret<(&'a str, usize)> {
        let Some(open) = self.toks.get(self.pos) else {
            return errf!("assembler: expected '{{'");
        };
        if !matches!(open.tok, AsmTok::LBrace) {
            return errf!("assembler: expected '{{'");
        }
        let mut depth = 0usize;
        let mut j = self.pos;
        while j < self.toks.len() {
            match self.toks[j].tok {
                AsmTok::LBrace => depth += 1,
                AsmTok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        let inner = &self.src[open.end..self.toks[j].start];
                        return Ok((inner, j + 1));
                    }
                }
                _ => {}
            }
            j += 1;
        }
        errf!("assembler: unclosed '{{'")
    }
}
