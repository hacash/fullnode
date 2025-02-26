

#[allow(dead_code)]
struct BiddingRecord {
    time: u64,
    number: u32,
    diamond: DiamondName,
    addr: Address,
    fee: Amount,
}

#[allow(dead_code)]
#[derive(Default)]
struct BiddingProve {
    // dia number => bidding info
    latest: u32,
    failures: HashMap<u32, HashSet<Address>>, // The address of the broadcast invalid block
    biddings: HashMap<u32, VecDeque<BiddingRecord>>,
}





#[allow(dead_code)]
impl BiddingProve {

    const DELAY_SECS: usize = 15; 
    const RECORD_NUM: usize = 10; 
    const PROVE_HOLD: usize = 5;  // latest 5 diamonds

    fn failure(&mut self, dianum: u32, blk: &BlockPkg) {
        let coinbase = &blk.objc.as_ref().transactions()[0];
        let fails = self.failures.entry(dianum).or_default();
        fails.insert(coinbase.main());
    }

    fn record(&mut self, tx: &dyn TransactionRead, act: &DiamondMint) {
        let dianum = *act.d.number;
        if dianum > self.latest {
            self.latest = dianum; // update
        }
        let tnow = curtimes();
        let record = BiddingRecord {
            time: tnow,
            number: dianum,
            diamond: act.d.diamond,
            addr: tx.main(),
            fee: tx.fee().clone(),
        };

        macro_rules! rcdshow { () => {
            // println!("- devtest record bidding {} {}", &record.addr.readable(), &record.fee);  
            // print!(",{}({})", &record.addr.readable()[0..7], &record.fee);       
        }}
        let bids = self.biddings.entry(dianum).or_default();
        // push
        if bids.is_empty() {
            rcdshow!();
            bids.push_front(record); // push at first
            return
        }
        if record.fee <= bids[0].fee {
            return // no need to record lowwer
        }
        rcdshow!();
        if bids[0].time == record.time {
            bids[0] = record; // replace in same second
            return 
        }
        bids.push_front(record); // push at first
        let max = Self::DELAY_SECS + Self::RECORD_NUM;
        bids.truncate(max);
        // ok

    }

    fn check_fail(&self, dianum: u32, fee: Amount) -> Amount {
        let Some(fails) = self.failures.get(&dianum) else {
            return fee // no fail
        };
        let fsub = |x|fee.sub(&Amount::small(x, 247), AmtMode::U64).unwrap_or_default(); // -= 0.x
        match fails.len() {
            0..3 => fee,
            3 => fsub(5), // -= 0.5
            4 => fsub(9), // -= 0.9
            5.. => Amount::zero() // do not check
        }
    }

    fn highest(&self, dianum: u32, sta: &dyn State) -> Option<Amount> {
        let coresta = CoreStateRead::wrap(sta);
        let ttx = curtimes() - Self::DELAY_SECS as u64;
        if let Some(bids) = self.biddings.get(&dianum) {
            for r in bids.iter() {
                if r.number == dianum && r.time < ttx {
                    let hacbls = coresta.balance(&r.addr).unwrap_or_default();
                    if hacbls.hacash >= r.fee {
                        let rfe = self.check_fail(dianum, r.fee.clone());
                        return Some(rfe); // highest valid bid
                    }
                }
            }
        }
        // not find
        None
    }

    fn print(&self, dianum: u32) -> String {
        let mut items = String::new();
        items.push_str(&format!("MinterRecordBiddingList {} (\n", dianum));
        if let Some(bids) = self.biddings.get(&dianum) {
            for r in bids.iter() {
                let mut adr = r.addr.readable();
                let _ = adr.split_off(9);
                items.push_str(&format!("    {} {} {}... {}\n", 
                    timeshow(r.time).split_off(11), r.diamond.to_readable(), adr, r.fee));
            }
        }
        items.push_str(")");
        items
    }

    fn print_all(&self, _: u32) -> String {
        let strs: Vec<_> = self.biddings.keys().map(|a|self.print(*a)).collect();
        strs.join("\n")
    }

    fn roll(&mut self, dianum: u32) {
        let ph = Self::PROVE_HOLD as u32;
        if dianum <= ph {
            return
        }
        let expired = dianum - ph;
        self.failures.remove(&expired);
        self.biddings.remove(&expired);
    }
}