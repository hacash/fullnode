
#[derive(Clone, Copy)]
pub struct MintConf {
    pub chain_id: u64, // sub chain id
    pub sync_maxh: u64, // sync block of max height
    pub show_miner_name: bool,
    pub difficulty_adjust_blocks: u64, // height : 288
    pub each_block_target_time: u64, // secs : 300
    pub test_coin: bool
    // pub _test_mul: u64,
}



impl MintConf {

    pub fn is_mainnet(&self) -> bool {
        self.chain_id == 0
    }

    pub fn new(ini: &IniObj) -> MintConf {

        let sec = ini_section(ini, "mint");

        let cnf = MintConf {
            chain_id: ini_must_u64(&sec, "chain_id", 0),
            sync_maxh: ini_must_u64(&sec, "height_max", 0),
            show_miner_name: ini_must_bool(&sec, "show_miner_name", false),
            difficulty_adjust_blocks: ini_must_u64(&sec, "difficulty_adjust_blocks", 288), // 1 day
            each_block_target_time: ini_must_u64(&sec, "each_block_target_time", 300), // 5 mins
            test_coin: ini_must_bool(&sec, "test_coin", false),
            // _test_mul: ini_must_u64(&sec, "_test_mul", 1), // test
        };

        cnf
    }


}
