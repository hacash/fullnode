
combi_struct!{ AssetAmt,
    serial: Fold64
    amount: Fold64
}

impl std::fmt::Display for AssetAmt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f,"({}:{})", self.serial, self.amount)
    }
}

impl std::cmp::Ord for AssetAmt {
    fn cmp(&self, other: &Self) -> Ordering {
        assert!(*self.serial == *other.serial);
        self.amount.cmp(&other.amount)
    }
}

impl std::cmp::PartialOrd for AssetAmt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}


macro_rules! checked_opt {
    ($name : ident) => {
        pub fn $name(&self, other: &Self) -> Ret<Self> {
            let err = errf!("cannot do {} with asset {} and {}", stringify!($name), self, other);
            if *self.serial != *other.serial {
                return err
            }
            match (*self.amount).$name(*other.amount) {
                Some(v) => Self{
                    serial: self.serial,
                    amount: Fold64::from(v)?,
                }.checked(),
                _ => err,
            }
        }
        
    };
}

impl AssetAmt {

    checked_opt!{ checked_add }
    checked_opt!{ checked_sub }

    pub fn checked(self) -> Ret<Self> {
        Ok(Self{
            serial: self.serial.checked()?,
            amount: self.amount.checked()?,
        })
    }

}



impl AssetAmt {

    pub fn new(serial: Fold64) -> Self {
        Self {
            serial,
            ..Default::default()
        }
    }

    pub fn from(s: u64, a: u64) -> Ret<Self> {
        Ok(Self {
            serial: Fold64::from(s)?,
            amount: Fold64::from(a)?,
        })
    }

}


