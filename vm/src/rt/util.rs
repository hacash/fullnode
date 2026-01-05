
pub fn local_operand_param_parse(mark: u8) -> (String, u8) {
    let opt = mark >> 6; // 0b00000011
    let idx = mark & 0b00111111; // max=64
    (match opt {
        0 => "+=",
        1 => "-=",
        2 => "*=",
        3 => "/=",
        _ => unreachable!()
    }.to_owned(), idx)
}


pub fn local_logic_param_parse(mark: u8) -> (String, u8) {
    let opt = mark >> 5; // 0b00000111
    let idx = mark & 0b00011111; // max=32
    (match opt {
        0 => "&&",
        1 => "||",
        2 => "==",
        3 => "!=",
        4 => ">",
        5 => ">=",
        6 => "<",
        7 => "<=",
        _ => unreachable!()
    }.to_owned(), idx)
}


pub fn ascii_show_string(s: &[u8]) -> Option<String> {
    match s.iter().any(|&a|a!=10&&(a<32||a>126)) {
        false => Some(String::from_utf8(s.to_vec()).unwrap()),
        true => None,
    }
}



