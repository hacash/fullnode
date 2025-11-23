




#[derive(Clone)]
pub struct EngineConf {
    pub max_block_txs: usize,
    pub max_block_size: usize,
    pub max_tx_size: usize,
    pub max_tx_actions: usize,
    pub chain_id: u32, // sub chain id
    pub unstable_block: u64, // The number of blocks that are likely to fall back from the fork
    pub fast_sync: bool,
    pub sync_maxh: u64, // sync max height, limit
    pub data_dir: String,
    pub block_data_dir: PathBuf, // block data
    pub state_data_dir: PathBuf, // chain state
    pub blogs_data_dir: PathBuf, // logs  state
    // block logs
    pub vmlogs_enable: bool,
    pub vmlogs_open_height: u64,
    pub vmlogs_can_delete: bool,
    // dev count
    pub dev_count_switch: usize,
    // data service
    pub diamond_form: bool,
    pub recent_blocks: bool,
    pub average_fee_purity: bool,
    pub lowest_fee_purity: u64, 
    // hac miner
    pub miner_enable: bool,
    pub miner_reward_address: Address,
    pub miner_message: Fixed16,
    // diamond miner
    pub dmer_enable: bool,
    pub dmer_reward_address: Address,
    pub dmer_bid_account: Account,
    pub dmer_bid_min:  Amount,
    pub dmer_bid_max:  Amount,
    pub dmer_bid_step: Amount,
    // tx pool
    pub txpool_maxs: Vec<usize>,
}


impl EngineConf {

    pub fn is_open_miner(&self) -> bool {
        self.miner_enable || self.dmer_enable
    }

    pub fn is_mainnet(&self) -> bool {
        self.chain_id == 0
    }
    
    pub fn new(ini: &IniObj, dbv: u32) -> EngineConf {
        

        // datadir
        let data_dir = get_mainnet_data_dir(ini);
    
        let mut state_data_dir = join_path(&data_dir, "state");
        state_data_dir.push(format!("v{}", dbv));

        // server sec
        let sec_server = &ini_section(ini, "server");

        // a simple hac trs size is 166 bytes
        const LOWEST_FEE_PURITY: u64 = 10000_00 / (166 / GSCU); // 1:244 = 1000000:238 / (166 / 32) = 200000

        let mut cnf = EngineConf{
            max_block_txs: 1000,
            max_block_size: 1024*1024*1, // 1MB
            max_tx_size: 1024 * 16, // 16kb
            max_tx_actions: 200, // 200
            chain_id: 0,
            unstable_block: 4, // 4 block
            fast_sync: false,
            sync_maxh: 0,
            block_data_dir: join_path(&data_dir, "block"),
            state_data_dir: state_data_dir,
            blogs_data_dir: join_path(&data_dir, "logs"),
            data_dir: data_dir.to_str().unwrap().to_owned(),
            dev_count_switch: 0,
            // logs
            vmlogs_enable: false,
            vmlogs_open_height: 0,
            vmlogs_can_delete: false,
            //
            diamond_form: ini_must_bool(sec_server, "diamond_form", true),
            recent_blocks: ini_must_bool(sec_server, "recent_blocks", false),
            average_fee_purity: ini_must_bool(sec_server, "average_fee_purity", false),
            lowest_fee_purity: LOWEST_FEE_PURITY,
            // HAC miner
            miner_enable: false,
            miner_reward_address: Address::default(),
            miner_message: Fixed16::default(),
            // Diamond miner
            dmer_enable: false,
            dmer_reward_address: Address::default(),
            dmer_bid_account: Account::create_by_password("123456").unwrap(),
            dmer_bid_min:  Amount::small_mei(1),
            dmer_bid_max:  Amount::small_mei(31),
            dmer_bid_step: Amount::small(5, 247),
            // tx pool
            txpool_maxs: Vec::default(),
        };
        // setup lowest_fee
        if ini_must(sec_server, "lowest_fee", "").len() > 0 {
            let lfepr = ini_must_amount(sec_server, "lowest_fee").compress(2, AmtCpr::Grow)
                .unwrap().to_238_u64().unwrap() / 166; //  =6024, simple hac trs size
            cnf.lowest_fee_purity = lfepr;
            println!("[Config] Node accepted lowest fee purity {}.", lfepr);
        }

        let sec = &ini_section(ini, "node");
        cnf.fast_sync = ini_must_bool(sec, "fast_sync", false);

        let sec_mint = &ini_section(ini, "mint");
        cnf.chain_id = ini_must_u64(sec_mint, "chain_id", 0) as u32;
        cnf.sync_maxh = ini_must_u64(sec_mint, "height_max", 0);
        cnf.dev_count_switch = ini_must_u64(sec_mint, "dev_count_switch", 0) as usize;

        let sec_vmlog = &ini_section(ini, "vmlog");
        cnf.vmlogs_enable = ini_must_bool(sec_vmlog, "enable", false);
        cnf.vmlogs_can_delete = ini_must_bool(sec_vmlog, "can_delete", false);
        cnf.vmlogs_open_height = ini_must_u64(sec_vmlog, "open_height", 0) as u64;

        // HAC miner
        let sec_miner = &ini_section(ini, "miner");
        cnf.miner_enable = ini_must_bool(sec_miner, "enable", false);
        if cnf.miner_enable {
            cnf.miner_reward_address = ini_must_address(sec_miner, "reward");
            let msg = ini_must_maxlen(sec_miner, "message", "", 16);
            let msgapp = vec![' ' as u8].repeat(16-msg.len());
            let msg: [u8; 16] = vec![msg.as_bytes().to_vec(), msgapp].concat().try_into().unwrap();
            cnf.miner_message = Fixed16::from_readable(&msg).unwrap();
        }

        // Diamond miner
        let sec_dmer = &ini_section(ini, "diamondminer");
        cnf.dmer_enable = ini_must_bool(sec_dmer, "enable", false);
        if cnf.dmer_enable {
            cnf.dmer_reward_address = ini_must_address(sec_dmer, "reward");
            cnf.dmer_bid_account = ini_must_account(sec_dmer, "bid_password");
            cnf.dmer_bid_min =  ini_must_amount(sec_dmer, "bid_min").compress(2, AmtCpr::Grow).unwrap();
            cnf.dmer_bid_max =  ini_must_amount(sec_dmer, "bid_max").compress(2, AmtCpr::Grow).unwrap();
            cnf.dmer_bid_step = ini_must_amount(sec_dmer, "bid_step").compress(2, AmtCpr::Grow).unwrap();
        }

        // tx pool
        let sec_txpool = &ini_section(ini, "txpool");
        cnf.txpool_maxs = ini_must(sec_txpool, "maxs", "").replace(" ", "").split(",").map(|a|{
            match a.parse::<usize>() {
                Ok(n) => n,
                _ => 100,
            }
        }).collect();

        // ok
        cnf
    }
    
}


