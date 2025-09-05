




pub const BLACKHOLE_ADDR: Address = Address::from([0u8; 21]);

fn blackhole_engulf(sta: &mut CoreState, addr: &Address) {
    if *addr != BLACKHOLE_ADDR {
        return
    }
    // set balance = empty
    sta.balance_set(addr, &Balance::new());
}


