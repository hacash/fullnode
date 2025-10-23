



pub const BLACKHOLE_ADDR: Address = ADDRESS_ZERO;

#[inline(always)]
fn blackhole_engulf(sta: &mut CoreState, addr: &Address) {
    if *addr == BLACKHOLE_ADDR {
        // set balance = empty
        sta.balance_set(addr, &Balance::new());
    }
}


