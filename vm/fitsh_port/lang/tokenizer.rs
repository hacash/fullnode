#[allow(dead_code)]
#[derive(Default)]
pub struct Tokenizer<'a> {
    texts: &'a [u8],
    idx: usize,
    tokens: Vec<Token>,
}

#[allow(dead_code)]
impl Tokenizer<'_> {
    pub fn new<'a>(texts: &'a [u8]) -> Tokenizer<'a> {
        Tokenizer {
            texts,
            ..Default::default()
        }
    }

    fn parse_num_bytes_or_address(s: &str) -> Ret<Token> {
        if s.starts_with("0x") {
            // Ox1AE23F
            let v = s.to_owned().split_off(2);
            return Ok(match hex::decode(v) {
                Ok(d) => Bytes(d),
                _ => return errf!("hex data format invalid '{}'", s),
            });
        } else if s.starts_with("0b") {
            // 0b11110000
            let e = errf!("binary data '{}' format invalid", s);
            let v = s.to_owned().split_off(2);
            let vl = v.len();
            if vl == 0 || vl % 8 != 0 || vl > 128 {
                return e;
            }
            let n = vl / 8;
            return Ok(match u128::from_str_radix(&v, 2) {
                Ok(d) => Bytes(d.to_be_bytes()[16 - n..].to_vec()),
                _ => return e,
            });
        } else if let Some(addr) = Self::parse_address(s) {
            // address
            return Ok(addr);
        }
        // maybe integer
        Ok(Integer(match s.parse::<u128>() {
            Ok(u) => u,
            _ => return errf!("integer type parse failed for '{}'", s),
        }))
    }

    fn parse_address(s: &str) -> Option<Token> {
        let sl = s.len();
        if sl < 30 || sl > 34 {
            return None;
        }
        match FieldAddress::from_readable(s) {
            Ok(a) => Some(Address(a)),
            _ => None,
        }
    }

    fn parse_hex_nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }

    fn parse_escape_byte(&self, max: usize, err_msg: &str) -> Ret<(u8, usize)> {
        let nxt = self.idx + 1;
        if nxt >= max {
            return Err(err_msg.to_string().into());
        }
        let esc = self.texts[nxt];
        let parsed = match esc {
            b'0' => (0u8, 2usize),
            b't' => (b'\t', 2),
            b'n' => (b'\n', 2),
            b'r' => (b'\r', 2),
            b'b' => (0x08, 2),
            b'f' => (0x0c, 2),
            b'v' => (0x0b, 2),
            b'\\' => (b'\\', 2),
            b'\'' => (b'\'', 2),
            b'"' => (b'"', 2),
            b'x' => {
                if nxt + 2 >= max {
                    return Err(err_msg.to_string().into());
                }
                let hi = Self::parse_hex_nibble(self.texts[nxt + 1]);
                let lo = Self::parse_hex_nibble(self.texts[nxt + 2]);
                match (hi, lo) {
                    (Some(hi), Some(lo)) => ((hi << 4) | lo, 4),
                    _ => return Err(err_msg.to_string().into()),
                }
            }
            _ => return Err(err_msg.to_string().into()),
        };
        Ok(parsed)
    }

    pub fn parse_comments(&mut self, max: usize) -> Ret<bool> {
        let c = self.texts[self.idx] as char;
        macro_rules! gtc {
            ($n: expr) => {
                self.texts[self.idx + $n] as char
            };
        }
        match c {
            '/' => {
                // single line comments
                self.idx += 1;
                while self.idx < max && gtc!(0) != '\n' {
                    self.idx += 1;
                }
                Ok(true)
            }
            '*' => {
                // nested multiple line comments
                self.idx += 1;
                let mut depth = 1usize;
                while self.idx < max {
                    if self.idx + 1 < max && gtc!(0) == '/' && gtc!(1) == '*' {
                        self.idx += 2;
                        depth += 1;
                        continue;
                    }
                    if self.idx + 1 < max && gtc!(0) == '*' && gtc!(1) == '/' {
                        self.idx += 2;
                        depth -= 1;
                        if depth == 0 {
                            return Ok(true);
                        }
                        continue;
                    }
                    self.idx += 1;
                }
                errf!("unterminated block comment")
            }
            _ => Ok(false),
        }
    }

    pub fn parse_identifier(&mut self, max: usize, c: char) -> Rerr {
        let mut s = String::new();
        s.push(c);
        while self.idx < max {
            let c = self.texts[self.idx] as char;
            match c {
                '0'..='9' | 'a'..='z' | 'A'..='Z' | '$' | '_' => s.push(c),
                _ => break,
            }
            self.idx += 1;
        }
        let ok = Ok(());
        if let Ok(k) = KwTy::build(&s) {
            self.tokens.push(Keyword(k));
            return ok;
        }
        self.tokens.push(match Self::parse_address(&s) {
            Some(addr) => addr,
            _ => Identifier(s.clone()),
        });
        ok
    }

    pub fn parse_number(&mut self, max: usize, c: char) -> Rerr {
        let mut s = String::new();
        s.push(c);
        while self.idx < max {
            let c = self.texts[self.idx] as char;
            match c {
                '0'..='9' | 'a'..='z' | 'A'..='Z' | '_' => s.push(c),
                _ => break,
            }
            self.idx += 1;
        }

        // Check for type suffix format: number + type (e.g., 100u8, 1000_u64)
        let type_suffixes = ["u8", "u16", "u32", "u64", "u128"];
        let mut num_part = s.clone();
        let mut suffix_kw: Option<KwTy> = None;

        for suffix in &type_suffixes {
            if s.ends_with(suffix) {
                let without_suffix = &s[..s.len() - suffix.len()];
                let without_underscores: String =
                    without_suffix.chars().filter(|c| *c != '_').collect();
                if !without_underscores.is_empty() && without_underscores.parse::<u128>().is_ok() {
                    num_part = without_underscores;
                    if let Ok(kw) = KwTy::build(suffix) {
                        suffix_kw = Some(kw);
                    }
                    break;
                }
            }
        }

        // If no suffix, remove underscores from the entire string
        if suffix_kw.is_none() {
            let without_underscores: String = s.chars().filter(|c| *c != '_').collect();
            if !without_underscores.is_empty() {
                num_part = without_underscores;
            }
        }

        // Parse the number part first
        let token = Self::parse_num_bytes_or_address(&num_part)?;
        match (token, suffix_kw) {
            (Integer(n), Some(kw)) => self.tokens.push(IntegerWithSuffix(n, kw)),
            (token, Some(kw)) => {
                self.tokens.push(token);
                self.tokens.push(Keyword(kw));
            }
            (token, None) => self.tokens.push(token),
        }
        Ok(())
    }

    pub fn parse_symbol(&mut self, max: usize, c: char) -> Rerr {
        let mut s = String::new();
        s.push(c);
        while self.idx < max {
            let c = self.texts[self.idx] as char;
            match c {
                '+' | '-' | '*' | '/' | '=' | '!' | '.' | ':' | '>' | '<' | '|' | '&' | '%'
                | '^' => s.push(c),
                _ => break,
            }
            self.idx += 1;
        }
        let ok = Ok(());
        if let Ok(k) = KwTy::build(&s) {
            self.tokens.push(Keyword(k));
            return ok;
        }
        if let Ok(o) = OpTy::build(&s) {
            self.tokens.push(Operator(o));
            return ok;
        }
        errf!("unsupported symbol '{}'", s)
    }

    pub fn parse_bytes(&mut self, max: usize, _c: char) -> Rerr {
        let err_msg = "bytes format invalid";
        let mut s = vec![];
        let mut closed = false;
        while self.idx < max {
            let c = self.texts[self.idx] as char;
            if c == '\"' {
                closed = true;
                self.idx += 1;
                break;
            }
            if c == '\\' {
                let (byte, consumed) = self.parse_escape_byte(max, err_msg)?;
                s.push(byte);
                self.idx += consumed;
                continue;
            }
            if !c.is_ascii() {
                return errf!("{}", err_msg);
            }
            s.push(c as u8);
            self.idx += 1;
        }
        if !closed {
            return errf!("{}", err_msg);
        }
        self.tokens.push(Bytes(s));
        Ok(())
    }

    pub fn parse_char(&mut self, max: usize, _c: char) -> Rerr {
        let err_msg = "char format invalid";
        if self.idx >= max - 1 {
            return errf!("{}", err_msg);
        }

        let byte = if self.texts[self.idx] == b'\\' {
            let (byte, consumed) = self.parse_escape_byte(max, err_msg)?;
            self.idx += consumed;
            byte
        } else {
            let byte = self.texts[self.idx];
            self.idx += 1;
            if byte == b'\'' || !byte.is_ascii() {
                return errf!("{}", err_msg);
            }
            byte
        };

        if self.idx >= max || self.texts[self.idx] != b'\'' {
            return errf!("{}", err_msg);
        }
        self.idx += 1;

        self.tokens.push(Character(byte));
        Ok(())
    }

    pub fn parse(mut self) -> Ret<Vec<Token>> {
        // use TokenType::*;
        let max = self.texts.len();
        while self.idx < max {
            let c = self.texts[self.idx] as char;
            self.idx += 1;
            if c == '/' && self.idx < max {
                if self.parse_comments(max)? {
                    continue;
                }
            }
            match c {
                '0'..='9' => self.parse_number(max, c)?,
                'A'..='Z' | 'a'..='z' | '$' | '_' => self.parse_identifier(max, c)?,
                '{' | '}' | '(' | ')' | '[' | ']' => self.tokens.push(Partition(c)),
                // Comma is a soft separator token.
                // Semicolon is normalized to comma at lexical stage.
                ',' | ';' => self.tokens.push(Partition(',')),
                '+' | '-' | '*' | '/' | '=' | '!' | '.' | ':' | '>' | '<' | '|' | '&' | '%'
                | '^' => self.parse_symbol(max, c)?,
                '"' => self.parse_bytes(max, c)?,
                '\'' => self.parse_char(max, c)?,
                ' ' | '\n' | '\r' | '\t' => {} // ignore
                _ => return errf!("unsupported char [{}]", c),
            }
        }
        Ok(self.tokens)
    }
}
