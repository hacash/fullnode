



pub const BLACKHOLE_ADDR: Address = ADDRESS_ZERO;

fn blackhole_engulf(sta: &mut CoreState, addr: &Address) {
    if *addr != BLACKHOLE_ADDR {
        return
    }
    // set balance = empty
    sta.balance_set(addr, &Balance::new());
}


