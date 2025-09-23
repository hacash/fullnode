
#[allow(dead_code)]
#[derive(Default)]
pub struct Tokenizer<'a> {
    texts: &'a[u8],
    tokty: TokenType,
    idx: usize,
    token: String,
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

    fn parse_num_or_bytes(s: &str) -> Ret<Token> {
        if s.starts_with("0x") {
            // Ox1AE23F
            let v = s.to_owned().split_off(2);
            return Ok(match hex::decode(v) {
                Ok(d) => Bytes(d),
                _ => return errf!("hex data format error '{}'", s),
            })
        } else if s.starts_with("0b") || s.len() >= 10 {
            // 0b11110000
            let e = errf!("binary data '{}' format error ", s);
            let v = s.to_owned().split_off(2);
            let vl = v.len();
            if vl % 8 != 0 {
                return e
            }
            let n = vl / 8;
            return Ok(match u128::from_str_radix(&v, 2) {
                Ok(d) => Bytes(d.to_be_bytes()[16-n..].to_vec()),
                _ => return e,
            })
        }else if let Some(addr) = Self::parse_address(s) {
            // address
            return Ok(addr)
        }
        // maybe uint
        Ok(Integer(match s.parse::<u128>() {
            Ok(u) => u,
            _ => return errf!("parse Integer type error for '{}'", s),
        }))
    }

    fn parse_address(s: &str) -> Option<Token> {
        let sl = s.len();
        if sl < 30 || sl > 34 {
            return None
        }
        match Address::from_readable(s) {
            Ok(a) => Some(Addr(a)),
            _ => None,
        }
    }

    fn conv(s: &String) -> Ret<Token> {
        let c = s.as_bytes()[0] as char;
        Ok(match c {
            '0'..='9' => Self::parse_num_or_bytes(s)?,
            '_'|'$'|'a'..='z'|'A'..='Z' =>  match KwTy::build(s) {
                Ok(k) => Keyword(k),
                _ => match Self::parse_address(s) {
                    Some(addr) => addr,
                    _ => Identifier(s.clone()),
                }
            },
            '\"' => Bytes({
                let mut d = s.as_bytes()[1..].to_vec();
                d.pop(); d
            }),
            '{'|'}'|'('|')'|'['|']' => Partition(c),
            _ => match KwTy::build(s) {
                Ok(k) => Keyword(k),
                _ => Operator(OpTy::build(s)?)
            },
        })
    }

    fn push(&mut self, t: TokenType, c: char) -> Rerr {
        let tk = &mut self.token;
        let tks = &mut self.tokens;
        let ty = &mut self.tokty;
        if *ty == StrEsc { // \" \t \n
            tk.push(match c {
                't' => '\t',
                'n' => '\n',
                'r' => '\r',
                '\\' => '\\',
                a => a,
            });
            *ty = Str;
        }else if *ty == Str && c=='\\' { // mod
            *ty = StrEsc; // next
        }else if *ty == Str || t==Str {
            tk.push(c);
            if  *ty==Str && c=='"' { // end
                tks.push(Self::conv(tk)?); // end
                tk.clear();
                *ty = Blank;
            }else{
                *ty = Str;
            }
        }else if t == Blank || t == Split {
            if tk.len() > 0 {
                tks.push(Self::conv(tk)?);
                tk.clear();
            }
            if t == Split {
                tk.push(c);
                tks.push(Self::conv(tk)?);
                tk.clear();
            }
        }else if *ty==Number && (c=='x'||c=='b') {
            // 0b... or 0x...
            tk.push(c);
        }else if *ty==Number && (c=='.') {
            // 1.25 or 0.34
            tk.push(c);
        }else if *ty==Word && (c>='0'&&c<='9') {
            // foo123 or bar456
            tk.push(c);
        }else if t == *ty {
            tk.push(c);
        }else{
            // next
            if tk.len() > 0 {
                tks.push(Self::conv(tk)?);
                tk.clear();
            }
            tk.push(c);
            *ty = t;
        }
        Ok(())
    }

    pub fn parse(mut self) -> Ret<Vec<Token>> {
        use TokenType::*;
        let max = self.texts.len();
        while self.idx < max {
            let c = self.texts[self.idx] as char;
            match c {
                ' '|','|';'|'\n'|'\r'|'\t'   => self.push(Blank, c)?,
                '_'|'$'|'a'..='z'|'A'..='Z'  => self.push(Word, c)?,
                '0'..='9'                    => self.push(Number, c)?,
                '('|')'|'{'|'}'|'['|']'      => self.push(Split, c)?,
                '+'|'-'|'*'|'/'|'='|'!'|
                '.'|':'|
                '>'|'<'|'|'|'&'|'%'|'^'      => self.push(Symbol, c)?,
                '"'|'\\'                     => self.push(Str, c)?,
                _ => return errf!("unsupport char [{}]", c) 
            }
            // next
            self.idx += 1;
        }
        Ok(self.tokens)
    } 

}


