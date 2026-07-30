
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LxOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl LxOp {
    fn bits(self) -> u8 {
        match self {
            LxOp::Add => 0,
            LxOp::Sub => 1,
            LxOp::Mul => 2,
            LxOp::Div => 3,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            LxOp::Add => "+=",
            LxOp::Sub => "-=",
            LxOp::Mul => "*=",
            LxOp::Div => "/=",
        }
    }

    fn from_bits(opt: u8) -> Self {
        match opt {
            0 => LxOp::Add,
            1 => LxOp::Sub,
            2 => LxOp::Mul,
            3 => LxOp::Div,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LxLg {
    And,
    Or,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl LxLg {
    fn bits(self) -> u8 {
        match self {
            LxLg::And => 0,
            LxLg::Or => 1,
            LxLg::Eq => 2,
            LxLg::Ne => 3,
            LxLg::Gt => 4,
            LxLg::Ge => 5,
            LxLg::Lt => 6,
            LxLg::Le => 7,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            LxLg::And => "&&",
            LxLg::Or => "||",
            LxLg::Eq => "==",
            LxLg::Ne => "!=",
            LxLg::Gt => ">",
            LxLg::Ge => ">=",
            LxLg::Lt => "<",
            LxLg::Le => "<=",
        }
    }

    fn from_bits(opt: u8) -> Self {
        match opt {
            0 => LxLg::And,
            1 => LxLg::Or,
            2 => LxLg::Eq,
            3 => LxLg::Ne,
            4 => LxLg::Gt,
            5 => LxLg::Ge,
            6 => LxLg::Lt,
            7 => LxLg::Le,
            _ => unreachable!(),
        }
    }
}

pub const LXOP_MAX_IDX: u8 = 0b_0011_1111;
pub const LXLG_MAX_IDX: u8 = 0b_0001_1111;

pub fn encode_local_operand_mark(op: LxOp, idx: u8) -> VmrtRes<u8> {
    if idx > LXOP_MAX_IDX {
        return Err(ItrErr::new(
            ItrErrCode::InstParamsErr,
            &format!("local operand idx {} out of range {}", idx, LXOP_MAX_IDX),
        ))
    }
    Ok((op.bits() << 6) | idx)
}

pub fn encode_local_logic_mark(op: LxLg, idx: u8) -> VmrtRes<u8> {
    if idx > LXLG_MAX_IDX {
        return Err(ItrErr::new(
            ItrErrCode::InstParamsErr,
            &format!("local logic idx {} out of range {}", idx, LXLG_MAX_IDX),
        ))
    }
    Ok((op.bits() << 5) | idx)
}

pub fn decode_local_operand_mark(mark: u8) -> (LxOp, u8) {
    let opt = mark >> 6; // high 2 bits
    let idx = mark & LXOP_MAX_IDX; // low 6 bits, max=64
    (LxOp::from_bits(opt), idx)
}

pub fn decode_local_logic_mark(mark: u8) -> (LxLg, u8) {
    let opt = mark >> 5; // high 3 bits
    let idx = mark & LXLG_MAX_IDX; // low 5 bits, max=32
    (LxLg::from_bits(opt), idx)
}

pub fn local_operand_param_parse(mark: u8) -> (String, u8) {
    let (op, idx) = decode_local_operand_mark(mark);
    (op.symbol().to_owned(), idx)
}

pub fn local_logic_param_parse(mark: u8) -> (String, u8) {
    let (op, idx) = decode_local_logic_mark(mark);
    (op.symbol().to_owned(), idx)
}

