

#[allow(dead_code)]
struct BiddingRecord {
    time: u64,
    number: u32,
    diamond: DiamondName,
    addr: Address,
    fee: Amount,
}



#[allow(dead_code)]
impl HacashMinter {

    const DELAY_SECS: usize = 15; 
    const RECORD_NUM: usize = 10; 

    fn record_bidding(&self, tx: &dyn TransactionRead, act: &DiamondMint) {
        let tnow = curtimes();
        let record = BiddingRecord {
            time: tnow,
            number: *act.d.number,
            diamond: act.d.diamond,
            addr: tx.main(),
            fee: tx.fee().clone(),
        };
        macro_rules! rcdshow { () => {
            // println!("- devtest record bidding {} {}", &record.addr.readable(), &record.fee);       
        }}
        // push
        let mut bds = self.biddings.lock().unwrap();
        if bds.is_empty() {
            rcdshow!();
            (*bds).push_front(record); // push at first
            return
        }
        if record.fee <= bds[0].fee {
            return // no need to record lowwer
        }
        rcdshow!();
        if bds[0].time == record.time {
            (*bds)[0] = record; // replace in same second
            return 
        }
        (*bds).push_front(record); // push at first
        let max = Self::DELAY_SECS + Self::RECORD_NUM;
        (*bds).truncate(max);
        // ok

    }


    fn highest_bidding(&self, number: u32, sta: &dyn State) -> Amount {
        let coresta = CoreStateRead::wrap(sta);
        let ttx = curtimes() - Self::DELAY_SECS as u64;
        for r in self.biddings.lock().unwrap().iter() {
            if r.number == number && r.time < ttx {
                let hacbls = coresta.balance(&r.addr).unwrap_or_default();
                if hacbls.hacash >= r.fee {
                    return r.fee.clone() // highest valid bid
                }
            }
        }
        // 0:0
        Amount::zero()
    }

    fn print_bidding(&self) -> String {
        let mut items = String::new();
        items.push_str("MinterRecordBiddingList(\n");
        for r in self.biddings.lock().unwrap().iter() {
            let mut adr = r.addr.readable();
            let _ = adr.split_off(9);
            items.push_str(&format!("    {} {} {}... {}\n", 
                timeshow(r.time).split_off(11), r.diamond.to_readable(), adr, r.fee));
        }
        items.push_str(")");
        items
    }

    fn clear_bidding(&self) {
        let mut bds = self.biddings.lock().unwrap();
        (*bds).clear();
    }
}