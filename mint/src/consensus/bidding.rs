//! Diamond bidding state used internally by `HacashConsensus`.
//!
//! Ported from fullnodedev `mint/src/check/bidding.rs` without Engine Weak /
//! discover loops; node polls explicit deferred candidate batches.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use base::{
    BlkPkg, Block, BlockAdmissionDecision, CoreStateRead, DeferredId, PowBlockExt, StateRead, TxPkg,
};
use field::{Address, Amount, Hash};
use num_bigint::BigUint;
use sys::{Rerr, Ret, curtimes, errf};

use crate::action::diamond::DiamondMint;
use crate::difficulty::{biguint_to_hash, hash_bigger_than, u32_to_biguint};
use crate::minter::block_reward_number;

/// Low-bid tip held pending replay (after-execute reject code).
pub const LOW_BID_PENDING_ERR: &str = "mint.low_bid.pending";
/// Low-bid shadow cache branch/root capacity exhausted.
pub const LOW_BID_CACHE_FULL_ERR: &str = "mint.low_bid.cache_full";

fn hash_half(h: &Hash) -> String {
    let s = format!("{}", h);
    s[..16.min(s.len())].to_string()
}

#[derive(Clone)]
struct BiddingRecord {
    usable: bool,
    tarhei: u64,
    time: u64,
    txhx: Hash,
    addr: Address,
    fee: Amount,
}

struct BiddingBook {
    uniq_top: Vec<BiddingRecord>,
}

#[derive(Clone)]
struct LowBidBranch {
    root_fee: Amount,
    blocks: Vec<BlkPkg>,
}

impl LowBidBranch {
    fn create(root: BlkPkg, root_fee: Amount) -> Self {
        Self {
            root_fee,
            blocks: vec![root],
        }
    }

    fn len(&self) -> usize {
        self.blocks.len()
    }

    fn root_hash(&self) -> Hash {
        self.blocks[0].hash()
    }

    fn root_difficulty(&self) -> u32 {
        self.blocks[0].block().pow_difficulty()
    }

    fn tip_hash(&self) -> Hash {
        self.blocks.last().unwrap().hash()
    }

    fn contains(&self, hash: &Hash) -> bool {
        self.blocks.iter().any(|blk| blk.hash() == *hash)
    }

    fn parent_index(&self, prev: &Hash) -> Option<usize> {
        self.blocks.iter().position(|blk| blk.hash() == *prev)
    }

    fn push_child(&mut self, blk: BlkPkg) {
        self.blocks.push(blk);
    }

    fn fork_from_parent(&self, parent_idx: usize, blk: BlkPkg) -> Self {
        let mut blocks = self.blocks[..=parent_idx].to_vec();
        blocks.push(blk);
        Self {
            root_fee: self.root_fee.clone(),
            blocks,
        }
    }
}

struct LowBidGroup {
    dianum: u32,
    height: u64,
    started_at: Instant,
    branches: Vec<LowBidBranch>,
}

impl LowBidGroup {
    fn create(dianum: u32, root: BlkPkg, root_fee: Amount, started_at: Instant) -> Self {
        Self {
            dianum,
            height: root.height(),
            started_at,
            branches: vec![LowBidBranch::create(root, root_fee)],
        }
    }

    fn branch_num(&self) -> usize {
        self.branches.len()
    }

    fn has_hash(&self, hash: &Hash) -> bool {
        self.branches.iter().any(|branch| branch.contains(hash))
    }

    fn add_root(&mut self, root: BlkPkg, root_fee: Amount, max_branches: usize) -> bool {
        let hash = root.hash();
        if self.has_hash(&hash) {
            return true;
        }
        if self.branches.len() >= max_branches {
            println!(
                "[MintLowBid] group root full height={} diamond={} branches={} max_branches={}",
                self.height,
                self.dianum,
                self.branches.len(),
                max_branches,
            );
            return false;
        }
        self.branches.push(LowBidBranch::create(root, root_fee));
        true
    }

    fn matches_tip(&self, prev: &Hash) -> bool {
        self.branches
            .iter()
            .any(|branch| branch.tip_hash() == *prev)
    }

    fn try_cache_child(&mut self, blk: BlkPkg, max_len: usize, max_branches: usize) -> bool {
        let hash = blk.hash();
        if self.has_hash(&hash) {
            println!(
                "[MintLowBid] child already cached height={} hash={} group_height={}",
                blk.height(),
                hash_half(&hash),
                self.height,
            );
            return true;
        }
        let prev = blk.block().prev_hash();
        for idx in 0..self.branches.len() {
            let Some(parent_idx) = self.branches[idx].parent_index(&prev) else {
                continue;
            };
            let next_len = parent_idx + 2;
            if next_len > max_len {
                println!(
                    "[MintLowBid] child over limit group_height={} height={} hash={} prev={} next_len={} max_len={}",
                    self.height,
                    blk.height(),
                    hash_half(&hash),
                    hash_half(&prev),
                    next_len,
                    max_len,
                );
                return false;
            }
            if parent_idx + 1 == self.branches[idx].len() {
                self.branches[idx].push_child(blk);
            } else {
                if self.branches.len() >= max_branches {
                    println!(
                        "[MintLowBid] group branch full group_height={} height={} hash={} prev={} branches={} max_branches={}",
                        self.height,
                        blk.height(),
                        hash_half(&hash),
                        hash_half(&prev),
                        self.branches.len(),
                        max_branches,
                    );
                    return false;
                }
                let fork = self.branches[idx].fork_from_parent(parent_idx, blk);
                self.branches.push(fork);
            }
            println!(
                "[MintLowBid] child cached group_height={} diamond={} height={} hash={} prev={} branches={}",
                self.height,
                self.dianum,
                self.branches[idx.min(self.branches.len() - 1)]
                    .blocks
                    .last()
                    .unwrap()
                    .height(),
                hash_half(&hash),
                hash_half(&prev),
                self.branches.len(),
            );
            return true;
        }
        false
    }

    fn replay_branches(&self) -> Vec<LowBidBranch> {
        let mut branches = self.branches.clone();
        branches.sort_by(|a, b| {
            b.len()
                .cmp(&a.len())
                .then_with(|| b.root_fee.cmp(&a.root_fee))
                .then_with(|| a.root_hash().as_ref().cmp(b.root_hash().as_ref()))
        });
        branches
    }
}

struct DiamondBiddingInner {
    latest: u32,
    books: HashMap<u32, BiddingBook>,
    low_bid_groups: HashMap<u64, LowBidGroup>,
    replay_allow: HashMap<DeferredId, HashSet<Hash>>,
    block_arrive_time: HashMap<Hash, u64>,
    block_arrive_order: VecDeque<Hash>,
    max_shadow_len: usize,
    max_group_branches: usize,
}

impl DiamondBiddingInner {
    const DELAY_SECS: usize = 10;
    const HACD_KEEP: usize = 10;
    const UNIQ_TOP_MAX: usize = 50;
    const LOW_BID_KEEP_SECS: u64 = 2400; // 40 min
    const BLOCK_ARRIVE_KEEP: usize = 50;

    fn new(max_shadow_len: usize) -> Self {
        let max_shadow_len = max_shadow_len.max(1);
        Self {
            latest: 0,
            books: HashMap::new(),
            low_bid_groups: HashMap::new(),
            replay_allow: HashMap::new(),
            block_arrive_time: HashMap::new(),
            block_arrive_order: VecDeque::new(),
            max_shadow_len,
            max_group_branches: max_shadow_len,
        }
    }

    fn record(&mut self, curr_hei: u64, tx: &TxPkg, act: &DiamondMint) {
        let dianum = act.d.number.uint();
        if dianum > self.latest {
            self.latest = dianum;
        }
        let record = BiddingRecord {
            usable: true,
            tarhei: curr_hei / 5 * 5 + 5,
            time: curtimes(),
            txhx: tx.hash(),
            addr: tx.tx().main(),
            fee: tx.tx().fee().clone(),
        };
        let book = self.books.entry(dianum).or_insert_with(|| BiddingBook {
            uniq_top: Vec::new(),
        });
        let mut updated = false;
        for item in book.uniq_top.iter_mut() {
            if item.addr != record.addr {
                continue;
            }
            if record.fee >= item.fee {
                *item = record.clone();
            }
            updated = true;
            break;
        }
        if !updated {
            book.uniq_top.push(record);
        }
        book.uniq_top
            .sort_by(|a, b| b.fee.cmp(&a.fee).then_with(|| b.time.cmp(&a.time)));
        book.uniq_top.truncate(Self::UNIQ_TOP_MAX);
        self.trim_books();
    }

    fn highest(&self, curhei: u64, dianum: u32, sta: &dyn StateRead, fblkt: u64) -> Ret<Amount> {
        if fblkt == 0 {
            return Ok(Amount::zero());
        }
        let Some(book) = self.books.get(&dianum) else {
            return Ok(Amount::zero());
        };
        let coresta = CoreStateRead::wrap(sta);
        let ttx = fblkt.saturating_sub(Self::DELAY_SECS as u64);
        for r in book.uniq_top.iter() {
            let isusa = curhei <= r.tarhei || r.usable;
            if r.time < ttx && isusa {
                let hacbls = coresta.balance(&r.addr)?.unwrap_or_default();
                if hacbls.hacash >= r.fee {
                    return Ok(r.fee.clone());
                }
            }
        }
        Ok(Amount::zero())
    }

    fn mark_block_arrival(&mut self, hei: u64, hash: Hash) {
        if hei % 5 != 4 {
            return;
        }
        if self.block_arrive_time.contains_key(&hash) {
            return;
        }
        self.block_arrive_time.insert(hash, curtimes());
        self.block_arrive_order.push_back(hash);
        while self.block_arrive_order.len() > Self::BLOCK_ARRIVE_KEEP {
            let Some(hx) = self.block_arrive_order.pop_front() else {
                break;
            };
            self.block_arrive_time.remove(&hx);
        }
    }

    fn prev_block_arrive_time(&self, prevhx: &Hash) -> u64 {
        self.block_arrive_time.get(prevhx).copied().unwrap_or(0)
    }

    fn add_low_bid_root(&mut self, dianum: u32, blk: BlkPkg, root_fee: Amount) -> bool {
        let height = blk.height();
        let hash = blk.hash();
        match self.low_bid_groups.entry(height) {
            std::collections::hash_map::Entry::Occupied(mut ent) => {
                let group = ent.get_mut();
                if !group.add_root(blk, root_fee.clone(), self.max_group_branches) {
                    return false;
                }
                println!(
                    "[MintLowBid] root grouped height={} hash={} diamond={} branches={} release_in={}s fee={}",
                    height,
                    hash_half(&hash),
                    dianum,
                    group.branch_num(),
                    Self::LOW_BID_KEEP_SECS.saturating_sub(group.started_at.elapsed().as_secs()),
                    root_fee,
                );
                true
            }
            std::collections::hash_map::Entry::Vacant(ent) => {
                let started_at = Instant::now();
                ent.insert(LowBidGroup::create(
                    dianum,
                    blk,
                    root_fee.clone(),
                    started_at,
                ));
                println!(
                    "[MintLowBid] root pending height={} hash={} diamond={} branches=1 release_in={}s fee={}",
                    height,
                    hash_half(&hash),
                    dianum,
                    Self::LOW_BID_KEEP_SECS,
                    root_fee,
                );
                true
            }
        }
    }

    fn min_pow_hash_by_prev(&self, prev: &Hash) -> Option<[u8; 32]> {
        for group in self.low_bid_groups.values() {
            for branch in group.branches.iter() {
                if branch.tip_hash() != *prev {
                    continue;
                }
                let max_hash = u32_to_biguint(branch.root_difficulty()) * BigUint::from(4u32);
                return Some(biguint_to_hash(&max_hash));
            }
        }
        None
    }

    fn cache_low_bid_child(&mut self, blk: BlkPkg) -> Option<DeferredId> {
        let prev = blk.block().prev_hash();
        for group in self.low_bid_groups.values_mut() {
            if !group.matches_tip(&prev) {
                continue;
            }
            return group
                .try_cache_child(blk, self.max_shadow_len, self.max_group_branches)
                .then_some(DeferredId::new(group.height));
        }
        None
    }

    fn take_deferred_groups(&mut self, root_min: u64, head_max: u64) -> Vec<LowBidGroup> {
        self.low_bid_groups.retain(|_, group| {
            let keep = group.height >= root_min && group.height <= head_max;
            if !keep {
                println!(
                    "[MintLowBid] group dropped height={} diamond={} branches={} root_window=[{}, {}]",
                    group.height,
                    group.dianum,
                    group.branch_num(),
                    root_min,
                    head_max,
                );
            }
            keep
        });
        let mut heights = Vec::new();
        for (height, group) in self.low_bid_groups.iter() {
            if group.started_at.elapsed().as_secs() >= Self::LOW_BID_KEEP_SECS {
                heights.push(*height);
            }
        }
        heights.sort_unstable();
        let mut groups = Vec::with_capacity(heights.len());
        for height in heights {
            if let Some(group) = self.low_bid_groups.remove(&height) {
                groups.push(group);
            }
        }
        groups
    }

    fn allow_replay_batch(&mut self, id: DeferredId, candidates: &[Vec<BlkPkg>]) {
        let hashes = candidates
            .iter()
            .flatten()
            .map(BlkPkg::hash)
            .collect::<HashSet<_>>();
        self.replay_allow.insert(id, hashes);
    }

    fn clear_replay_batch(&mut self, id: DeferredId) {
        self.replay_allow.remove(&id);
    }

    fn is_replay_allowed(&self, hash: &Hash) -> bool {
        self.replay_allow
            .values()
            .any(|hashes| hashes.contains(hash))
    }

    fn remove_tx(&mut self, dianum: u32, hx: Hash) {
        let Some(book) = self.books.get_mut(&dianum) else {
            return;
        };
        for item in book.uniq_top.iter_mut() {
            if item.txhx == hx {
                item.usable = false;
            }
        }
    }

    fn roll(&mut self, dianum: u32) {
        if dianum > self.latest {
            self.latest = dianum;
        }
        self.trim_books();
    }

    fn trim_books(&mut self) {
        let keep_from = self.latest.saturating_sub(Self::HACD_KEEP as u32 - 1);
        self.books.retain(|num, _| *num >= keep_from);
    }

    fn pending_count(&self) -> usize {
        self.low_bid_groups
            .values()
            .map(|g| g.branches.iter().map(|b| b.len()).sum::<usize>())
            .sum()
    }
}

/// Diamond bidding / low-bid shadow cache (public name kept for exports).
pub struct DiamondBidding {
    inner: Mutex<DiamondBiddingInner>,
}

impl Default for DiamondBidding {
    fn default() -> Self {
        Self::new(40)
    }
}

impl DiamondBidding {
    pub fn new(max_shadow_len: usize) -> Self {
        Self {
            inner: Mutex::new(DiamondBiddingInner::new(max_shadow_len)),
        }
    }

    pub fn record(&self, curr_hei: u64, tx: &TxPkg, act: &DiamondMint) {
        self.inner.lock().unwrap().record(curr_hei, tx, act);
    }

    pub fn highest(
        &self,
        curhei: u64,
        dianum: u32,
        sta: &dyn StateRead,
        fblkt: u64,
    ) -> Ret<Amount> {
        self.inner
            .lock()
            .unwrap()
            .highest(curhei, dianum, sta, fblkt)
    }

    pub fn mark_block_arrival(&self, hei: u64, hash: Hash) {
        self.inner.lock().unwrap().mark_block_arrival(hei, hash);
    }

    pub fn prev_block_arrive_time(&self, prevhx: &Hash) -> u64 {
        self.inner.lock().unwrap().prev_block_arrive_time(prevhx)
    }

    pub fn add_low_bid_root(&self, dianum: u32, blk: BlkPkg, root_fee: Amount) -> bool {
        self.inner
            .lock()
            .unwrap()
            .add_low_bid_root(dianum, blk, root_fee)
    }

    pub fn min_pow_hash_by_prev(&self, prev: &Hash) -> Option<[u8; 32]> {
        self.inner.lock().unwrap().min_pow_hash_by_prev(prev)
    }

    pub fn cache_low_bid_child(&self, blk: BlkPkg) -> Option<DeferredId> {
        self.inner.lock().unwrap().cache_low_bid_child(blk)
    }

    pub fn remove_tx(&self, dianum: u32, hx: Hash) {
        self.inner.lock().unwrap().remove_tx(dianum, hx);
    }

    pub fn roll(&self, dianum: u32) {
        self.inner.lock().unwrap().roll(dianum);
    }

    pub fn pending_count(&self) -> usize {
        self.inner.lock().unwrap().pending_count()
    }

    /// Pull ready low-bid groups (after 40min), emit **all** sorted branches
    /// (best first) so node can try the next branch if an earlier one fails —
    /// matches fullnodedev `replay_low_bid_group` multi-branch fallback under
    /// the pull-based deferred-batch model.
    pub fn drain_deferred_batches(
        &self,
        root_min: u64,
        head_max: u64,
    ) -> Vec<(DeferredId, Vec<Vec<BlkPkg>>)> {
        let mut inner = self.inner.lock().unwrap();
        let groups = inner.take_deferred_groups(root_min, head_max);
        let mut out = Vec::new();
        for group in groups {
            let branches = group.replay_branches();
            if branches.is_empty() {
                continue;
            }
            println!(
                "[MintLowBid] drain begin height={} diamond={} branches={}",
                group.height,
                group.dianum,
                branches.len(),
            );
            let mut group_out = Vec::new();
            for (i, branch) in branches.into_iter().enumerate() {
                println!(
                    "[MintLowBid] drain branch#{} height={} diamond={} selected_len={} root_hash={} root_fee={}",
                    i,
                    group.height,
                    group.dianum,
                    branch.len(),
                    hash_half(&branch.root_hash()),
                    branch.root_fee,
                );
                group_out.push(branch.blocks);
            }
            let id = DeferredId::new(group.height);
            inner.allow_replay_batch(id, &group_out);
            out.push((id, group_out));
        }
        out
    }

    pub fn finish_deferred_batch(&self, id: DeferredId) {
        self.inner.lock().unwrap().clear_replay_batch(id);
    }

    pub fn check_admission(&self, pkg: &BlkPkg) -> Ret<BlockAdmissionDecision> {
        let mut inner = self.inner.lock().unwrap();
        if inner.is_replay_allowed(&pkg.hash()) {
            return Ok(BlockAdmissionDecision::Continue);
        }
        let prev = pkg.block().prev_hash();
        if let Some(min_pow) = inner.min_pow_hash_by_prev(&prev) {
            let pow = pkg.block().hash().into_array();
            if hash_bigger_than(&pow, &min_pow) {
                return errf!("low-bid tip child PoW hash exceeds 4x root difficulty fence");
            }
            if let Some(id) = inner.cache_low_bid_child(pkg.clone()) {
                return Ok(BlockAdmissionDecision::Defer(id));
            }
            return errf!("low-bid tip child rejected (cache full or orphan)");
        }
        Ok(BlockAdmissionDecision::Continue)
    }

    /// Highest-bid gate after execute (OLD `check_highest_bid_of_block`).
    pub fn check_highest_bid(&self, pkg: &BlkPkg, prev_state: &dyn StateRead) -> Rerr {
        use crate::action::diamond::DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST;
        use crate::action::util::pickout_diamond_mint_action_from_block;

        let curhei = pkg.height();
        if curhei % 5 != 0 {
            return Ok(());
        }
        let block = pkg.block();
        let Some((tidx, txp, diamint)) = pickout_diamond_mint_action_from_block(block) else {
            return Ok(());
        };
        const CKN: u32 = DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST;
        if tidx != 1 && curhei > 600_000 {
            return errf!("diamond mint transaction must be the first tx in block");
        }
        let dianum = diamint.d.number.uint();
        let bidfee = txp.fee().clone();
        // Min bidding fee (same rule as block_check::check_diamond_mint_minimum_bidding_fee).
        let bidmin = Amount::mei(block_reward_number(curhei) as u64);
        if bidfee < bidmin && dianum > CKN {
            return errf!(
                "diamond bidding fee {} cannot be less than {} after number {}",
                bidfee,
                bidmin,
                CKN
            );
        }
        let mut bidrecord = self.inner.lock().unwrap();
        let t4blkt = bidrecord.prev_block_arrive_time(&block.prev_hash());
        let rhbf = bidrecord.highest(curhei, dianum, prev_state, t4blkt)?;
        if bidfee < rhbf {
            if dianum > CKN {
                if bidrecord.is_replay_allowed(&pkg.hash()) {
                    println!(
                        "[MintLowBid] replay low bid accepted height={} hash={} diamond={} fee={} fence={}",
                        curhei,
                        hash_half(&pkg.hash()),
                        dianum,
                        bidfee,
                        rhbf,
                    );
                } else {
                    if !bidrecord.add_low_bid_root(dianum, pkg.clone(), bidfee.clone()) {
                        return errf!("{}", LOW_BID_CACHE_FULL_ERR);
                    }
                    println!(
                        "[MintLowBid] low root detected height={} hash={} diamond={} fee={} fence={}",
                        curhei,
                        hash_half(&pkg.hash()),
                        dianum,
                        bidfee,
                        rhbf,
                    );
                    return errf!("{}", LOW_BID_PENDING_ERR);
                }
            }
        }
        bidrecord.remove_tx(dianum, txp.hash());
        bidrecord.roll(dianum);
        Ok(())
    }

    pub fn on_stable_block(&self, block: &dyn Block) {
        // Stable tip: trim fee-book entries older than keep window via roll side-effect.
        let _ = block.height();
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use base::{BlkPkg, Block, PkgOrigin, PkgSource, TxRef};
    use field::{Amount, Encode, Hash};

    use super::{DiamondBidding, LowBidGroup};

    #[derive(Debug)]
    struct TestBlock {
        height: u64,
        hash: Hash,
    }

    impl Encode for TestBlock {
        fn size(&self) -> usize {
            40
        }

        fn encode_to(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(&self.height.to_be_bytes());
            out.extend_from_slice(self.hash.as_bytes());
        }
    }

    impl Block for TestBlock {
        fn version(&self) -> u8 {
            1
        }

        fn height(&self) -> u64 {
            self.height
        }

        fn hash(&self) -> Hash {
            self.hash
        }

        fn prev_hash(&self) -> Hash {
            Hash::default()
        }

        fn mrklroot(&self) -> Hash {
            Hash::default()
        }

        fn timestamp(&self) -> u64 {
            1
        }

        fn transactions(&self) -> &[TxRef] {
            &[]
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn test_pkg(height: u64, byte: u8) -> BlkPkg {
        BlkPkg::from_block(
            Arc::new(TestBlock {
                height,
                hash: Hash::from([byte; 32]),
            }),
            PkgSource::new(PkgOrigin::Broadcast),
        )
    }

    #[test]
    fn deferred_replay_permission_is_internal_and_batch_scoped() {
        let bidding = DiamondBidding::new(4);
        let pkg = test_pkg(5, 7);
        bidding.inner.lock().unwrap().low_bid_groups.insert(
            pkg.height(),
            LowBidGroup::create(
                1,
                pkg.clone(),
                Amount::zero(),
                Instant::now() - Duration::from_secs(2_400),
            ),
        );

        let batches = bidding.drain_deferred_batches(1, 10);
        assert_eq!(batches.len(), 1);
        let id = batches[0].0;
        assert!(bidding.inner.lock().unwrap().is_replay_allowed(&pkg.hash()));

        bidding.finish_deferred_batch(id);
        assert!(!bidding.inner.lock().unwrap().is_replay_allowed(&pkg.hash()));
    }
}
