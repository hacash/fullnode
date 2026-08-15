//! `HacashConsensus`

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base::{
    BlkPkg, Block, BlockAdmissionDecision, BlockBuild, BlockProducer, BlockRef, ChainView,
    Consensus, ConsensusNodeHooks, CoreStateRead, Engine, Node, Peer, PkgOrigin, PkgSource,
    PowBlockBuild, PowBlockExt, StateChunkRef, StateLayer, Transaction, TransactionBuild,
    TxGroupId, TxOrdering, TxPkg, TxPolicy, TxPool, TxPoolGroupSpec, TxRef,
};
use field::{Address, Amount, Encode, Fixed16, Hash, Timestamp, Uint1, Uint4};
use protocol::block_std::{StdBlock, calculate_mrkl_prelude_modify, calculate_mrkl_prelude_update};
use protocol::tx_std::TransactionType2;
use sys::{Rerr, Ret, Waiter};

use crate::MintConf;
use crate::action::diamond::DiamondMint;
use crate::bidding::DiamondBidding;
use crate::block_check;
use crate::consensus::params::mint_params_for;
use crate::difficulty::{DifficultyConfig, DifficultyGnr, LOWEST_DIFFICULTY, u32_to_hash};
use crate::tx_coinbase::{CoinbaseExtend, CoinbaseExtendDataV1, CoinbaseTx};

/// Genesis-state marker for the immutable diamond ownership representation.
/// The value is exactly one byte: `0` or `1`.
pub const DIAMOND_FORM_STATE_KEY: &[u8] = b"_consensus.diamond_form";

const BLOCK_REWARD_STEP_BLOCK: u64 = 100_000;
const BLOCK_REWARD_DEF_LIST: [u8; 66] = [
    1, 1, 2, 3, 5, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1,
];

pub fn block_hasher(height: u64, data: &[u8]) -> [u8; 32] {
    x16rs::block_hash(height, data)
}

pub fn block_reward_number(block_height: u64) -> u8 {
    let curstp = block_height / BLOCK_REWARD_STEP_BLOCK;
    if curstp >= BLOCK_REWARD_DEF_LIST.len() as u64 {
        return 1;
    }
    BLOCK_REWARD_DEF_LIST[curstp as usize]
}

pub fn cumulative_block_reward(block_height: u64) -> u64 {
    let mut remain = block_height.saturating_add(1);
    let mut total = 0u64;
    for &reward in BLOCK_REWARD_DEF_LIST.iter() {
        let blocks = remain.min(BLOCK_REWARD_STEP_BLOCK);
        total = total.saturating_add(blocks.saturating_mul(reward as u64));
        remain = remain.saturating_sub(blocks);
        if remain == 0 {
            break;
        }
    }
    total.saturating_sub(1)
}

pub struct HacashConsensus {
    genesis: BlockRef,
    diamond_form_flag: u64,
    mint_conf: MintConf,
    bidding: DiamondBidding,
    difficulty: DifficultyGnr,
    miner: MinerConf,
    miner_pending: Mutex<VecDeque<MinerBlockStuff>>,
    miner_packing: Mutex<()>,
    miner_notice_count: Arc<AtomicU64>,
    started: std::sync::atomic::AtomicBool,
}

#[derive(Clone)]
pub struct MinerConf {
    pub enable: bool,
    pub reward: Address,
    pub message: Fixed16,
    pub diamond_enable: bool,
    pub diamond_reward: Address,
    pub diamond_bid_account: sys::Account,
    pub diamond_bid_min: Amount,
    pub diamond_bid_max: Amount,
    pub diamond_bid_step: Amount,
}

impl Default for MinerConf {
    fn default() -> Self {
        Self {
            enable: false,
            reward: Address::default(),
            message: Fixed16::default(),
            diamond_enable: false,
            diamond_reward: Address::default(),
            diamond_bid_account: sys::Account::create_by_password("123456").unwrap(),
            diamond_bid_min: Amount::mei(1).compress(2, field::AmtCpr::Grow).unwrap(),
            diamond_bid_max: Amount::mei(31).compress(2, field::AmtCpr::Grow).unwrap(),
            diamond_bid_step: Amount::coin(5, 247)
                .compress(2, field::AmtCpr::Grow)
                .unwrap(),
        }
    }
}

#[derive(Clone)]
struct MinerBlockStuff {
    block: StdBlock,
    coinbase_tx: CoinbaseTx,
    coinbase_nonce: Hash,
    target_hash: Hash,
    mrkl_modify_list: Vec<Hash>,
}

pub struct MinerPendingWork {
    pub height: u64,
    pub coinbase_nonce: Hash,
    pub block_intro: Vec<u8>,
    pub target_hash: Hash,
    pub version: u8,
    pub prevhash: Hash,
    pub timestamp: u64,
    pub transaction_count: u32,
    pub reward_address: Address,
    pub transaction_body_list: Vec<Vec<u8>>,
    pub coinbase_body: Vec<u8>,
    pub mkrl_modify_list: Vec<Hash>,
}

impl HacashConsensus {
    pub const TX_GROUP_NORMAL: TxGroupId = TxGroupId::DEFAULT;
    pub const TX_GROUP_DIAMOND_MINT: TxGroupId = TxGroupId::new(1);

    /// Business relay channel bit advertised in the P2P services mask for
    /// diamond-mint tx relay. The node layer never names this bit; it is
    /// declared here via `TxPolicy::tx_pool_groups` and aggregated
    /// into the local peer's advertised services, and matched against peer
    /// service masks when deciding whether to relay diamond-mint txs.
    ///
    /// Bit 3 matches the historical `NODE_DIAMOND = 1 << 3` so existing
    /// mainnet peers remain wire-compatible.
    pub const SERVICE_BIT_DIAMOND_RELAY: u64 = 1 << 3;

    pub fn new(services: &dyn base::ExecutionServices) -> Ret<Self> {
        Self::with_miner(services, MinerConf::default())
    }

    pub fn with_miner(services: &dyn base::ExecutionServices, miner: MinerConf) -> Ret<Self> {
        Self::with_config(services, MintConf::default(), miner)
    }

    pub fn with_config(
        services: &dyn base::ExecutionServices,
        mint: MintConf,
        miner: MinerConf,
    ) -> Ret<Self> {
        let diamond_form_flag = protocol::execution_params(services)?.diamond_form_flag;
        let mint_params = mint_params_for(mint.chain_id);
        let diff_cfg = DifficultyConfig::from_mint_params(mint.chain_id, mint_params);
        let max_shadow = diff_cfg.difficulty_group_blocks.saturating_mul(10).max(1) as usize;
        Ok(Self {
            genesis: crate::genesis::genesis_block(),
            diamond_form_flag,
            mint_conf: mint,
            bidding: DiamondBidding::new(max_shadow),
            difficulty: DifficultyGnr::new(diff_cfg),
            miner,
            miner_pending: Mutex::new(VecDeque::new()),
            miner_packing: Mutex::new(()),
            miner_notice_count: Arc::new(AtomicU64::new(0)),
            started: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn mint_conf(&self) -> &MintConf {
        &self.mint_conf
    }

    pub fn pending_replay_count(&self) -> usize {
        self.bidding.pending_count()
    }

    pub fn miner_enabled(&self) -> bool {
        self.miner.enable
    }

    pub fn diamond_miner_enabled(&self) -> bool {
        self.miner.diamond_enable
    }

    pub fn diamond_miner_bid_address(&self) -> Address {
        Address::from(*self.miner.diamond_bid_account.address())
    }

    pub fn diamond_miner_reward_address(&self) -> Address {
        self.miner.diamond_reward
    }

    pub fn miner_notice_count(&self) -> u64 {
        self.miner_notice_count.load(Ordering::Relaxed)
    }

    pub fn miner_notice_wait(
        &self,
        view: &dyn ChainView,
        target_height: u64,
        wait_secs: u64,
    ) -> u64 {
        struct NoticeGuard {
            count: Arc<AtomicU64>,
        }
        impl Drop for NoticeGuard {
            fn drop(&mut self) {
                self.count.fetch_sub(1, Ordering::Relaxed);
            }
        }

        self.miner_notice_count.fetch_add(1, Ordering::Relaxed);
        let _guard = NoticeGuard {
            count: self.miner_notice_count.clone(),
        };
        let wait_secs = wait_secs.clamp(1, 300);
        let start = Instant::now();
        let wait = Duration::from_secs(wait_secs);
        let poll = Duration::from_millis(250);
        loop {
            let current_height = view.latest_height();
            if target_height > 0 && current_height >= target_height {
                return current_height;
            }
            if start.elapsed() >= wait {
                return current_height;
            }
            std::thread::sleep(poll.min(wait.saturating_sub(start.elapsed())));
        }
    }

    /// Async long-poll for miner workers (does not block tokio worker threads).
    pub async fn miner_notice_wait_async(
        &self,
        view: &dyn ChainView,
        target_height: u64,
        wait_secs: u64,
    ) -> u64 {
        struct NoticeGuard {
            count: Arc<AtomicU64>,
        }
        impl Drop for NoticeGuard {
            fn drop(&mut self) {
                self.count.fetch_sub(1, Ordering::Relaxed);
            }
        }

        self.miner_notice_count.fetch_add(1, Ordering::Relaxed);
        let _guard = NoticeGuard {
            count: self.miner_notice_count.clone(),
        };
        let wait_secs = wait_secs.clamp(1, 300);
        let start = Instant::now();
        let wait = Duration::from_secs(wait_secs);
        let poll = Duration::from_millis(250);
        loop {
            let current_height = view.latest_height();
            if target_height > 0 && current_height >= target_height {
                return current_height;
            }
            if start.elapsed() >= wait {
                return current_height;
            }
            let sleep_for = poll.min(wait.saturating_sub(start.elapsed()));
            tokio::time::sleep(sleep_for).await;
        }
    }

    fn target_hash_for_difficulty(diff: u32) -> Hash {
        let mut target = u32_to_hash(diff);
        for i in (0..target.len()).rev() {
            if target[i] > 0 {
                target[i] -= 1;
                break;
            }
            target[i] = 255;
        }
        Hash::from(target)
    }

    #[allow(dead_code)]
    fn check_pow(block: &dyn Block) -> Rerr {
        let got = block.hash();
        let target = Self::target_hash_for_difficulty(block.pow_difficulty());
        if got.as_ref() > target.as_ref() {
            return sys::errf!(
                "block difficulty check failed: expected at most {} but got {}",
                target,
                got
            );
        }
        Ok(())
    }

    fn increase_nonce(nonce: &mut Hash) -> Rerr {
        let mut bytes = nonce.into_array();
        for i in (0..bytes.len()).rev() {
            let (next, carry) = bytes[i].overflowing_add(1);
            bytes[i] = next;
            if !carry {
                *nonce = Hash::from(bytes);
                return Ok(());
            }
        }
        sys::errf!("coinbase nonce overflow")
    }

    fn update_coinbase_nonce(
        block: &mut StdBlock,
        coinbase_tx: &mut CoinbaseTx,
        nonce: Hash,
        mrkl_modify_list: &[Hash],
    ) -> Rerr {
        coinbase_tx.set_mining_nonce(nonce);
        let cbhx = coinbase_tx.hash();
        let mrklroot = calculate_mrkl_prelude_update(cbhx, mrkl_modify_list);
        block.set_mrklroot(mrklroot);
        Ok(())
    }

    fn create_pending_block(&self, engine: &dyn Engine, txpool: &dyn TxPool) -> sys::Ret<()> {
        let Some(block_ref) = self.build_next_block(engine, txpool)? else {
            return sys::errf!("miner not enabled");
        };
        let Some(block) = block_ref.as_any().downcast_ref::<StdBlock>().cloned() else {
            return sys::errf!("next block type is not standard block");
        };
        let Some(coinbase_tx) = block
            .transactions()
            .first()
            .and_then(|tx| tx.as_any().downcast_ref::<CoinbaseTx>())
            .cloned()
        else {
            return sys::errf!("next block coinbase transaction missing");
        };
        let mut pending = self.miner_pending.lock().unwrap();
        pending.push_front(MinerBlockStuff {
            target_hash: Self::target_hash_for_difficulty(block.pow_difficulty()),
            mrkl_modify_list: calculate_mrkl_prelude_modify(&block.transaction_hash_list(true)),
            block,
            coinbase_tx,
            coinbase_nonce: Hash::default(),
        });
        while pending.len() > 3 {
            pending.pop_back();
        }
        Ok(())
    }

    pub fn miner_pending_work(
        &self,
        engine: &dyn Engine,
        txpool: &dyn TxPool,
    ) -> sys::Ret<MinerPendingWork> {
        if !self.miner.enable {
            return sys::errf!("miner not enabled");
        }
        let latest_height = engine.latest_height();
        let need_create = {
            let pending = self.miner_pending.lock().unwrap();
            pending
                .front()
                .map_or(true, |stuff| stuff.block.height() <= latest_height)
        };
        if need_create {
            let _guard = self.miner_packing.lock().unwrap();
            let still_need_create = {
                let pending = self.miner_pending.lock().unwrap();
                pending
                    .front()
                    .map_or(true, |stuff| stuff.block.height() <= latest_height)
            };
            if still_need_create {
                self.create_pending_block(engine, txpool)?;
            }
        }
        let mut pending = self.miner_pending.lock().unwrap();
        let Some(stuff) = pending.front_mut() else {
            return sys::errf!("pending block not ready");
        };
        Self::increase_nonce(&mut stuff.coinbase_nonce)?;
        Self::update_coinbase_nonce(
            &mut stuff.block,
            &mut stuff.coinbase_tx,
            stuff.coinbase_nonce,
            &stuff.mrkl_modify_list,
        )?;
        stuff
            .block
            .replace_transaction(0, Arc::new(stuff.coinbase_tx.clone()))?;
        Ok(MinerPendingWork {
            height: stuff.block.height(),
            coinbase_nonce: stuff.coinbase_nonce,
            block_intro: stuff.block.encode_intro(),
            target_hash: stuff.target_hash,
            version: stuff.block.version(),
            prevhash: stuff.block.prev_hash(),
            timestamp: stuff.block.timestamp(),
            transaction_count: stuff.block.transaction_count().saturating_sub(1),
            reward_address: stuff.coinbase_tx.address,
            transaction_body_list: stuff
                .block
                .transactions()
                .iter()
                .map(|tx| tx.encode())
                .collect(),
            coinbase_body: stuff.coinbase_tx.encode(),
            mkrl_modify_list: stuff.mrkl_modify_list.clone(),
        })
    }

    pub fn miner_success_block(
        &self,
        reg: &dyn base::BinaryCodecs,
        height: u64,
        block_nonce: u32,
        coinbase_nonce: Hash,
    ) -> sys::Ret<BlkPkg> {
        if !self.miner.enable {
            return sys::errf!("miner not enabled");
        }
        let (mut block, mut coinbase_tx, target_hash, mrkl_modify_list) = {
            let pending = self.miner_pending.lock().unwrap();
            let Some(stuff) = pending.iter().find(|stuff| stuff.block.height() == height) else {
                return sys::errf!("pending block height {} not found", height);
            };
            (
                stuff.block.clone(),
                stuff.coinbase_tx.clone(),
                stuff.target_hash,
                stuff.mrkl_modify_list.clone(),
            )
        };
        block.set_nonce(block_nonce);
        Self::update_coinbase_nonce(
            &mut block,
            &mut coinbase_tx,
            coinbase_nonce,
            &mrkl_modify_list,
        )?;
        block.replace_transaction(0, Arc::new(coinbase_tx))?;
        let block_hash = block.hash();
        if block_hash.as_ref() > target_hash.as_ref() {
            return sys::errf!(
                "difficulty check failed: expected at most {} but got {}",
                target_hash,
                block_hash
            );
        }
        let data = block.encode();
        let pkg = BlkPkg::from_bytes(reg, data, PkgSource::new(PkgOrigin::Mining))?;
        Ok(pkg)
    }

    /// Remove a pending block only after the node accepted the submission.
    /// Keeping it on admission failure allows the miner to retry the result.
    pub fn miner_mark_block_submitted(&self, height: u64) {
        self.miner_pending
            .lock()
            .unwrap()
            .retain(|stuff| stuff.block.height() != height);
    }

    pub fn diamond_miner_success_tx(
        &self,
        reg: &dyn base::BinaryCodecs,
        view: &dyn ChainView,
        txpool: &dyn TxPool,
        node: &dyn Node,
        action_body: Vec<u8>,
    ) -> sys::Ret<TxPkg> {
        if !self.miner.diamond_enable {
            return sys::errf!("diamond miner not enabled");
        }
        let (act_ref, used) =
            crate::action::diamond::create_diamond_mint(reg, DiamondMint::KIND, &action_body)?;
        if used != action_body.len() {
            return sys::errf!("diamond mint action trailing bytes");
        }
        let Some(mint) = act_ref.as_any().downcast_ref::<DiamondMint>() else {
            return sys::errf!("diamond mint action type invalid");
        };

        let snapshot = view.optimistic_canonical().ok().flatten().ok_or_else(|| {
            sys::Error::fault("state changed during diamond mint tx creation")
                .with_code("state_changed")
        })?;
        let start_epoch = snapshot.epoch;
        let state = CoreStateRead::wrap(snapshot.view());
        let act = &mint.d;
        let mint_number = act.number.uint();
        let lastdia = state.latest_diamond()?.unwrap_or_default();
        if mint_number != lastdia.number.uint() + 1 {
            return sys::errf!("invalid diamond number");
        }
        if mint_number > 1 && act.prev_hash != lastdia.born_hash {
            return sys::errf!("invalid diamond prev hash");
        }
        if act.address != self.miner.diamond_reward {
            return sys::errf!("invalid diamond reward address");
        }

        let bid_addr = self.diamond_miner_bid_address();
        let mut bid_offer = self.miner.diamond_bid_min.clone();
        if let Some(fbtx) = txpool.first(self.tx_pool_group_for_diamond_mint()) {
            let Ok(hbfe) = fbtx.tx().fee().compress(2, field::AmtCpr::Grow) else {
                return sys::errf!("cannot compress leading bid fee");
            };
            if hbfe > self.miner.diamond_bid_max {
                bid_offer = self.miner.diamond_bid_max.clone();
            } else if hbfe > bid_offer {
                if fbtx.tx().main() == bid_addr {
                    bid_offer = hbfe;
                } else if let Ok(new_bid) = hbfe.add_mode_u64(&self.miner.diamond_bid_step) {
                    bid_offer = new_bid.compress(2, field::AmtCpr::Grow).unwrap_or(new_bid);
                    if bid_offer > self.miner.diamond_bid_max {
                        bid_offer = self.miner.diamond_bid_max.clone();
                    }
                }
            }
        }

        if !view.validate_optimistic(start_epoch) {
            return sys::errf!("state changed during diamond mint tx creation");
        }
        let mut tx = TransactionType2::new_by(bid_addr, bid_offer, sys::curtimes());
        tx.push_action(act_ref)?;
        tx.fill_sign_account(&self.miner.diamond_bid_account)?;
        let pkg = TxPkg::from_bytes(reg, tx.encode(), PkgSource::new(PkgOrigin::Mining))?;
        node.submit_transaction(&pkg, true, false)?;
        Ok(pkg)
    }

    fn tx_pool_group_for_diamond_mint(&self) -> TxGroupId {
        Self::TX_GROUP_DIAMOND_MINT
    }

    fn clean_invalid_diamond_mint_txs(&self, view: &dyn ChainView, txpool: &dyn TxPool) -> Rerr {
        let Some(snapshot) = view.optimistic_canonical().ok().flatten() else {
            return Ok(());
        };
        let start_epoch = snapshot.epoch;
        let state = CoreStateRead::wrap(snapshot.view());
        let curdn = match state.latest_diamond() {
            Ok(d) => d.unwrap_or_default().number.uint(),
            // Best-effort pool cleanup: a state read failure stops this
            // cleanup pass; it must not silently judge mint txs invalid.
            Err(e) => {
                eprintln!(
                    "[minter] clean_invalid_diamond_mint_txs state read failed: {}",
                    e
                );
                return Ok(());
            }
        };
        let nextdn = curdn + 1;
        if !view.validate_optimistic(start_epoch) {
            // txpool maintenance is best-effort (§15); a stale snapshot
            // leaves cleanup to the next packing pass.
            return Ok(());
        }
        let _ = txpool.retain(Self::TX_GROUP_DIAMOND_MINT, &mut |a: &TxPkg| {
            crate::action::util::get_diamond_mint_number(a.tx()) == nextdn
        });
        Ok(())
    }

    fn preview_next_difficulty(
        &self,
        next_height: u64,
        history: &dyn base::BlockHistory,
    ) -> Ret<Option<(u32, field::Hash)>> {
        let prev = match history.block_at_height(
            next_height
                .checked_sub(1)
                .ok_or_else(|| sys::Error::fault("no parent height for next difficulty preview"))?,
        ) {
            Ok(Some(block)) => block,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        };
        let blkt = sys::curtimes();
        let target = self.difficulty.target(
            prev.pow_difficulty(),
            prev.timestamp(),
            next_height,
            blkt,
            history,
        )?;
        Ok(Some((target.num, field::Hash::from(target.hash))))
    }
}

impl Consensus for HacashConsensus {
    fn name(&self) -> &str {
        "hacash-mainnet"
    }

    fn chain_id(&self) -> base::ChainId {
        self.mint_conf.chain_id
    }

    fn mint_params(&self) -> base::MintParams {
        mint_params_for(self.mint_conf.chain_id)
    }

    fn genesis_block(&self) -> BlockRef {
        self.genesis.clone()
    }

    fn initialize(&self, layer: &mut dyn StateLayer) -> Rerr {
        crate::initialize::initialize(layer, self.mint_conf.diamond_form)
    }

    fn validate_genesis_state(&self, state: &dyn base::StateRead, root_height: u64) -> Rerr {
        let expected = self.mint_conf.diamond_form as u8;
        match state.get(DIAMOND_FORM_STATE_KEY)? {
            Some(raw) if raw.len() == 1 && raw[0] <= 1 => {
                if raw[0] != expected {
                    return sys::errf!(
                        "diamond_form configuration mismatch: config={} state={}",
                        expected,
                        raw[0]
                    );
                }
                Ok(())
            }
            Some(_) => sys::errf!("diamond_form genesis state marker is corrupted"),
            None if root_height == 0 => Ok(()),
            None => sys::errf!(
                "diamond_form genesis state marker is missing at root height {}; refusing startup",
                root_height
            ),
        }
    }

    fn genesis_state_needs_rebuild(
        &self,
        state: &dyn base::StateRead,
        root_height: u64,
    ) -> Ret<bool> {
        let marker = state.get(DIAMOND_FORM_STATE_KEY)?;
        Ok(root_height == 0 && marker.is_none())
    }

    fn chain_flags(&self, _height: u64) -> u64 {
        if self.mint_conf.diamond_form {
            self.diamond_form_flag
        } else {
            0
        }
    }

    fn check_block_data(&self, data: &[u8], view: &dyn ChainView) -> Rerr {
        block_check::check_block_data(data, view)
    }

    fn check_block_arrive_data(&self, data: &[u8], view: &dyn ChainView) -> Rerr {
        block_check::check_block_arrive_data(&self.difficulty, data, view)
    }

    fn check_block_arrive(&self, pkg: &BlkPkg, view: &dyn ChainView, fast_sync: bool) -> Rerr {
        // Fast sync skips consensus validation: the linear-head invariant
        // and full state execution are its only guards. Custom mints that
        // need side effects here may override this and ignore the flag.
        if fast_sync {
            return Ok(());
        }
        block_check::check_block_arrive(&self.difficulty, &self.mint_conf, pkg, view)
    }

    /// Publish the arrival record only after the block is durably accepted
    /// (§6 of the engine error contract): orphans and unvalidated blocks
    /// never pollute the bidding map.
    fn on_block_accepted(&self, pkg: &BlkPkg, _view: &dyn ChainView) -> Rerr {
        self.bidding.mark_block_arrival(pkg.height(), pkg.hash());
        Ok(())
    }

    fn check_block_admission(
        &self,
        pkg: &base::BlkPkg,
        _view: &dyn ChainView,
        fast_sync: bool,
    ) -> sys::Ret<BlockAdmissionDecision> {
        if fast_sync {
            return Ok(BlockAdmissionDecision::Continue);
        }
        self.bidding.check_admission(pkg)
    }

    fn check_block_before_execute(
        &self,
        pkg: &base::BlkPkg,
        parent: &dyn Block,
        history: &dyn base::BlockHistory,
        fast_sync: bool,
    ) -> Rerr {
        if fast_sync {
            return Ok(());
        }
        block_check::check_block_before_execute(
            &self.mint_conf,
            &self.difficulty,
            pkg,
            parent,
            history,
        )
    }

    fn check_block_after_execute(
        &self,
        pkg: &base::BlkPkg,
        _new_state: &StateChunkRef,
        parent_state: &dyn base::StateRead,
        _view: &dyn ChainView,
        fast_sync: bool,
    ) -> Rerr {
        if fast_sync {
            return Ok(());
        }
        block_check::check_highest_bid(&self.bidding, pkg, parent_state)
    }

    fn on_stable_block(&self, block: &dyn Block, _view: &dyn ChainView) -> Rerr {
        self.bidding.on_stable_block(block);
        Ok(())
    }
}

impl base::ForkChoice for HacashConsensus {}

impl TxPolicy for HacashConsensus {
    fn check_tx(&self, view: &dyn ChainView, tx: &TxPkg) -> Rerr {
        block_check::check_tx(&self.bidding, view, tx)
    }

    fn failed_revalidation_can_remove(&self, tx: &dyn Transaction) -> bool {
        tx.ty() < 3
    }

    fn tx_pool_groups(&self) -> Vec<TxPoolGroupSpec> {
        let mut normal =
            TxPoolGroupSpec::new(Self::TX_GROUP_NORMAL, "normal", TxOrdering::FeePurity);
        normal.default_capacity = 2000;
        let mut diamond =
            TxPoolGroupSpec::new(Self::TX_GROUP_DIAMOND_MINT, "diamond_mint", TxOrdering::Fee);
        diamond.default_capacity = 100;
        diamond.revalidate_interval = None;
        diamond.relay_service_bit = Some(Self::SERVICE_BIT_DIAMOND_RELAY);
        vec![normal, diamond]
    }

    fn tx_pool_group(&self, tx: &TxPkg) -> TxGroupId {
        if tx
            .tx()
            .actions()
            .iter()
            .any(|act| act.as_any().is::<DiamondMint>())
        {
            Self::TX_GROUP_DIAMOND_MINT
        } else {
            Self::TX_GROUP_NORMAL
        }
    }

    fn on_txs_confirmed(
        &self,
        view: &dyn ChainView,
        txpool: &dyn TxPool,
        txs: Vec<Hash>,
        height: u64,
    ) {
        if height % 5 == 0 {
            if let Err(e) = self.clean_invalid_diamond_mint_txs(view, txpool) {
                // Best-effort pool maintenance (§10): record the failure and
                // keep the pool untouched; never judge mint txs invalid here.
                eprintln!("[minter] clean_invalid_diamond_mint_txs failed: {}", e);
            }
        }
        if txs.len() > 1 {
            let _ = txpool.drain(&txs[1..]);
        }
    }
}

impl BlockProducer for HacashConsensus {
    fn external_exec_author(&self) -> Address {
        if self.miner.enable {
            self.miner.reward
        } else {
            Address::default()
        }
    }

    fn build_next_block(
        &self,
        engine: &dyn Engine,
        txpool: &dyn TxPool,
    ) -> sys::Ret<Option<BlockRef>> {
        if !self.miner.enable {
            return Ok(None);
        }
        if !self.miner.reward.is_privkey() {
            return sys::errf!(
                "miner reward address {} must be PRIVAKEY type but got version {}",
                self.miner.reward.to_readable(),
                self.miner.reward.version()
            );
        }
        // §8.1 step 1 / §10: if the activity channel is owned by Sync,
        // Recovery, Stopping, or sync has been requested (sync_waiting),
        // return None (Busy) immediately.  Packing during sync only produces
        // stale work that the next sync batch will invalidate; packing during
        // Recovery would contend with the recovery writer.  This consults the
        // ActivityGate BEFORE taking the strict `state_canonical` session so
        // the miner never blocks a writer simply by attempting to pack.
        if engine.is_packing_inhibited() {
            return Ok(None);
        }

        // §8.1 step 2: copy txpool candidates WITHOUT holding StateGate so
        // admission cannot block the miner and the miner cannot block writers.
        // Snapshot the next height first so the diamond-mint sampling window
        // matches the strict session that follows.
        let pre_height = engine.latest_height().saturating_add(1);
        let coinbase_tx = Arc::new(CoinbaseTx {
            ty: Uint1::from(CoinbaseTx::TYPE),
            address: self.miner.reward,
            reward: Amount::mei(block_reward_number(pre_height) as u64),
            message: self.miner.message,
            extend: CoinbaseExtend::must(CoinbaseExtendDataV1 {
                miner_nonce: Hash::default(),
                witness_count: Uint1::default(),
            }),
        });
        let base_tx_size = coinbase_tx.size();
        let mut candidates = Vec::<TxRef>::new();
        let mut candidate_bytes = 0usize;
        const PACKING_CANDIDATE_HARD_CAP: usize = 4096;
        let mint_params = engine.consensus().mint_params();
        let max_candidates = if mint_params.max_block_txs == 0 {
            PACKING_CANDIDATE_HARD_CAP
        } else {
            mint_params
                .max_block_txs
                .saturating_sub(1)
                .min(PACKING_CANDIDATE_HARD_CAP)
        };
        let max_scanned = max_candidates.saturating_mul(4).max(64);
        let mut scanned = 0usize;
        let max_candidate_bytes = (mint_params.max_block_size > 0)
            .then(|| mint_params.max_block_size.saturating_sub(base_tx_size));
        // Hard bound: at most one diamond-mint tx per block (OLD
        // check/block_build.rs "pick one diamond mint tx"). The diamond group
        // is fee-descending, so the first pickable entry is the highest bid.
        // Later entries could not pass the cumulative pick anyway (prev_hash /
        // number checks), but excluding them here makes the bound explicit.
        let mut diamond_kept = false;
        let mut collect_candidate = |txpkg: &base::TxPkg| {
            if candidates.len() >= max_candidates || scanned >= max_scanned {
                return false;
            }
            let is_diamond = crate::action::util::pickout_diamond_mint_action(txpkg.tx()).is_some();
            if is_diamond && diamond_kept {
                return true;
            }
            scanned = scanned.saturating_add(1);
            let next_bytes = candidate_bytes.saturating_add(txpkg.tx().size());
            if max_candidate_bytes.is_some_and(|max| next_bytes > max) {
                // One oversized/stale high-fee entry must not hide every
                // smaller candidate behind it.  Continue only within the
                // separate scan bound above.
                return true;
            }
            candidate_bytes = next_bytes;
            candidates.push(txpkg.tx_ref());
            diamond_kept |= is_diamond;
            true
        };
        if pre_height > 0 && pre_height % 5 == 0 {
            txpool.iter(Self::TX_GROUP_DIAMOND_MINT, &mut collect_candidate)?;
        }
        txpool.iter(Self::TX_GROUP_NORMAL, &mut collect_candidate)?;
        drop(collect_candidate);

        // §8.1 steps 3-6: strict read session.  `state_canonical()` captures
        // head hash + height + branch snapshot under one read guard so root
        // persist cannot commit between the height read and the snapshot.
        let Some(session) = engine.state_canonical().ok().flatten() else {
            return Ok(None);
        };
        let head_hash = session.head_hash();
        let next_height = session.head_height().saturating_add(1);
        if pre_height != next_height {
            // The head changed while candidates were copied outside the
            // strict lock.  In particular, the diamond-mint group is sampled
            // only at every fifth height, so reusing the old group selection
            // could build from the wrong candidate class.  Abort this cycle;
            // the miner loop will take a fresh bounded snapshot next time.
            return Ok(None);
        }
        let latest = engine.latest_block();
        let mut blk = StdBlock::genesis(engine.services().block_hasher_fn());
        blk.height = field::Uint5::from(next_height);
        blk.prevhash = head_hash;
        blk.timestamp = Timestamp::from(sys::curtimes());

        let mut newdifn = latest.pow_difficulty();
        if newdifn == 0 {
            newdifn = LOWEST_DIFFICULTY;
        }
        if self.difficulty.is_asert_height(next_height) || !self.mint_conf.is_mainnet() {
            if let Some((diff, _)) =
                self.preview_next_difficulty(next_height, engine.block_history().as_ref())?
            {
                newdifn = diff;
            }
        }
        blk.difficulty = Uint4::from(newdifn);

        blk.push_transaction(coinbase_tx)?;

        // §8.1 step 5: cumulative tx execution on the same snapshot.
        let picked = engine.try_pick_pending_txs_on_session(
            &session,
            candidates,
            next_height,
            self.miner.reward,
            base_tx_size,
            mint_params.max_block_txs,
            mint_params.max_block_size,
        );
        if !engine.validate_optimistic(session.epoch()) {
            return Ok(None);
        }
        for tx in picked {
            blk.push_transaction(tx)?;
        }
        blk.update_mrklroot();

        drop(session);

        Ok(Some(Arc::new(blk)))
    }
}

impl ConsensusNodeHooks for HacashConsensus {
    fn on_p2p_connect(
        &self,
        peer: Arc<dyn Peer>,
        _engine: Arc<dyn Engine>,
        txpool: Arc<dyn TxPool>,
    ) -> Rerr {
        std::thread::spawn(move || {
            if let Some(txp) = txpool.first(Self::TX_GROUP_DIAMOND_MINT) {
                let _ = peer.send_msg(base::MSG_TX_SUBMIT, txp.data().as_ref().to_vec());
            }
        });
        Ok(())
    }

    fn poll_deferred_batches(&self, view: &dyn ChainView) -> Ret<Vec<base::DeferredBatch>> {
        let hist = view.block_history();
        // Read the stable height before draining so a read failure cannot
        // follow a batch removal; the engine digests the error and stops the
        // round without requeueing (§6.6).
        let root_min = hist.stable_height()?.saturating_add(1);
        let head_max = view.latest_height().saturating_add(1);
        Ok(self
            .bidding
            .drain_deferred_batches(root_min, head_max)
            .into_iter()
            .map(|(id, branches)| base::DeferredBatch {
                id,
                candidates: branches
                    .into_iter()
                    .map(|blocks| base::DeferredCandidate { blocks })
                    .collect(),
            })
            .collect())
    }

    fn on_deferred_batch_result(&self, id: base::DeferredId, _result: base::DeferredBatchResult) {
        self.bidding.finish_deferred_batch(id);
    }

    fn start(&self, _waiter: Waiter) -> Rerr {
        self.started
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn exit(&self) {
        self.started
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
