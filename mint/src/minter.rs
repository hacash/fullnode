

pub struct HacashMinter {
    gnsblk: Arc<dyn Block>,
}

impl HacashMinter {

    pub fn create(_: &IniObj) -> Self {
        Self {
            gnsblk: genesis_block_pkg().into_block().into(),
        }
    }

}


impl Minter for HacashMinter {

    fn init(&self, _: &IniObj) {
        // extend actions
        protocol::action::setup_extend_actions_try_create(empty_create);
    }

    fn genesis_block(&self) -> Arc<dyn Block> {
        self.gnsblk.clone()
    }

    fn initialize(&self, sta: &mut dyn State) -> Rerr {
        do_initialize(sta)
    }


}

// 

pub fn empty_create(_kind: u16, _buf: &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>> {
    Ok(None)
}



