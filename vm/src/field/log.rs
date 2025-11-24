
combi_struct!{ VmLog,
    // height: BlockHeight
    addr: Address
    topic0:  Value
    topic1:  Value
    topic2:  Value
    topic3:  Value
    data:    Value
}

impl VmLog {

    pub fn render(&self, append: &str) -> String {
        let mut res = format!(r#""address":"{}","topic0":"{}","topic1":"{}","topic2":"{}","topic3":"{}","data":"{}""#, 
            self.addr.readable(),
            self.topic0.raw().hex(),
            self.topic1.raw().hex(),
            self.topic2.raw().hex(),
            self.topic3.raw().hex(),
            self.data.raw().hex(),
        );
        if append != "" {
            res += &(s!(",") + append);
        } 
        res
    }

    pub fn new(addr: Address, mut tds: Vec<Value>) -> VmrtRes<Self> {
        let tl = tds.len();
        if tl < 2 {
            return itr_err_fmt!(LogError, "argv num need at least 2")
        }
        // check can store
        for a in &tds {
            a.canbe_store()?;
        }
        let mut tp = || tds.pop().unwrap();
        let mut log = Self {
            addr: addr.clone(),
            topic0: Value::nil(),
            topic1: Value::nil(),
            topic2: Value::nil(),
            topic3: Value::nil(),
            data: tp(),
        };
        match tl {
            2 => {},
            3 => { log.topic1 = tp(); }
            4 => { log.topic2 = tp(); log.topic1 = tp(); }
            5 => { log.topic3 = tp(); log.topic2 = tp(); log.topic1 = tp(); }
            _ => return itr_err_fmt!(LogError, "argv num need less and eq than 5")
        };
        log.topic0 = tp(); // data
        // finish
        Ok(log)
    }


}



