//! The fork tree: a Mutex-guarded {root, head}.
//!
//! Every chunk from the root down is strongly held (root owns `children`).
//! Optimistic readers pin the captured root; critical readers prevent root
//! movement for the lifetime of their execution.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use base::{BlockRef, DiskDB, ForkChoiceKey, StateChunkRef};
use field::Hash;
use sys::{Ret, errf};

pub struct Tree {
    inner: Mutex<Inner>,
}

struct Inner {
    /// The highest root whose state and root markers are durable.
    root: StateChunkRef,
    /// The highest root already assigned to an ordered persistence job.
    scheduled_root: StateChunkRef,
    head: StateChunkRef,
    /// Changes whenever the canonical head changes. Readers use it to reject
    /// work that is internally consistent but no longer builds on the head.
    epoch: u64,
}

/// Outcome of a successful insert. `roll` is set when the root must advance.
pub struct Inserted {
    pub is_head: bool,
    pub reorg: bool,
    pub roll: Option<RollJob>,
    pub confirmed_txs: Vec<Hash>,
    pub reverted_txs: Vec<Hash>,
}

#[derive(Default)]
struct CanonicalTxDiff {
    confirmed: Vec<Hash>,
    reverted: Vec<Hash>,
}

/// A root advance waiting to be persisted. `chain` is oldest-first, from the
/// block just above the old root up to and including the new root.
pub struct RollJob {
    pub expected_root_hash: Hash,
    pub expected_root_height: u64,
    pub new_root: StateChunkRef,
    pub chain: Vec<StateChunkRef>,
}

pub fn height_of(chunk: &StateChunkRef) -> u64 {
    chunk
        .block_height()
        .expect("fork tree contains a non-block chunk")
}

pub fn hash_of(chunk: &StateChunkRef) -> Hash {
    chunk
        .block_identity()
        .expect("fork tree contains an unidentified block")
        .hash
}

pub fn fork_key_of(chunk: &StateChunkRef) -> ForkChoiceKey {
    chunk
        .block_identity()
        .expect("fork tree contains an unidentified block")
        .fork_choice
        .clone()
}

impl Tree {
    pub fn new(disk: Arc<dyn DiskDB>, root_block: BlockRef) -> Self {
        let root = root_chunk(disk, root_block);
        Self {
            inner: Mutex::new(Inner {
                head: root.clone(),
                scheduled_root: root.clone(),
                root,
                epoch: 0,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("fork tree lock poisoned")
    }

    pub fn root_height(&self) -> u64 {
        height_of(&self.lock().root)
    }

    pub fn head_height(&self) -> u64 {
        height_of(&self.lock().head)
    }

    /// Head hash and height in one lock acquisition.
    pub fn head_tip(&self) -> (Hash, u64) {
        let head = self.lock().head.clone();
        (hash_of(&head), height_of(&head))
    }

    pub fn head_block(&self) -> BlockRef {
        self.lock()
            .head
            .block_identity()
            .expect("fork tree contains an unidentified block")
            .block
            .clone()
    }

    pub fn epoch(&self) -> u64 {
        self.lock().epoch
    }

    /// Canonical tip and the root pin that keeps its weak parent chain alive.
    pub fn head_snapshot(&self) -> (Hash, u64, u64, StateChunkRef, StateChunkRef) {
        let inner = self.lock();
        let head = inner.head.clone();
        (
            hash_of(&head),
            height_of(&head),
            inner.epoch,
            head,
            inner.root.clone(),
        )
    }

    #[cfg(test)]
    pub fn find(&self, hash: &Hash) -> Option<StateChunkRef> {
        self.lock().find(hash)
    }

    pub fn contains(&self, hash: &Hash) -> bool {
        self.lock().find(hash).is_some()
    }

    pub fn block_context(&self, hash: &Hash) -> Option<(BlockRef, ForkChoiceKey)> {
        self.lock().find(hash).map(|parent| {
            let identity = parent
                .block_identity()
                .expect("fork tree contains an unidentified block");
            (identity.block.clone(), identity.fork_choice.clone())
        })
    }

    /// Candidate ancestry from the current durable root through `hash`.
    /// Returned blocks are oldest-first and remain alive independently of the
    /// short tree lock.
    pub fn branch_blocks(&self, hash: &Hash) -> Option<Vec<BlockRef>> {
        let inner = self.lock();
        let mut cursor = inner.find(hash)?;
        let mut blocks = Vec::new();
        loop {
            blocks.push(
                cursor
                    .block_identity()
                    .expect("fork tree contains an unidentified block")
                    .block
                    .clone(),
            );
            if cursor.ptr_eq(&inner.root) {
                break;
            }
            cursor = cursor.parent()?;
        }
        blocks.reverse();
        Some(blocks)
    }

    /// Discard the whole tree and restart from a disk-backed root. Used at
    /// boot and when recovering from a failed sync.
    pub fn reset_root(&self, disk: Arc<dyn DiskDB>, root_block: BlockRef) {
        let mut inner = self.lock();
        let root = root_chunk(disk, root_block);
        inner.head = root.clone();
        inner.scheduled_root = root.clone();
        inner.root = root;
        inner.epoch = inner.epoch.wrapping_add(1);
    }

    /// Attach a fully executed block chunk. `unstable_window` is how many
    /// blocks may stay above the durable root before it advances.
    pub fn attach(
        &self,
        parent_hash: &Hash,
        chunk: StateChunkRef,
        unstable_window: u64,
    ) -> Ret<Inserted> {
        self.attach_inner(parent_hash, chunk, unstable_window, false)
    }

    /// Attach one fast-sync block only if it is a direct canonical extension.
    /// All checks happen before the chunk is frozen or published.
    pub fn attach_linear(
        &self,
        parent_hash: &Hash,
        chunk: StateChunkRef,
        unstable_window: u64,
    ) -> Ret<Inserted> {
        self.attach_inner(parent_hash, chunk, unstable_window, true)
    }

    fn attach_inner(
        &self,
        parent_hash: &Hash,
        chunk: StateChunkRef,
        unstable_window: u64,
        linear: bool,
    ) -> Ret<Inserted> {
        let mut inner = self.lock();
        let height = chunk.block_height()?;
        let identity = chunk.block_identity()?.clone();
        let hash = identity.hash;
        let fork_choice = identity.fork_choice;
        let parent = inner.find(parent_hash).ok_or_else(|| {
            format!(
                "parent block <{}, {:?}> not in tree",
                height - 1,
                parent_hash
            )
        })?;
        if height != height_of(&parent) + 1 {
            return errf!(
                "block height {} does not follow parent height {}",
                height,
                height_of(&parent)
            );
        }
        if inner.find(&hash).is_some() {
            return errf!("block <{}, {:?}> already in tree", height, hash);
        }
        if linear && !parent.ptr_eq(&inner.head) {
            return errf!("fast-sync block parent is not the canonical head");
        }
        let Some(chunk_parent) = chunk.parent() else {
            return errf!("block <{}, {:?}> has no execution parent", height, hash);
        };
        if !chunk_parent.ptr_eq(&parent) {
            return errf!(
                "block <{}, {:?}> execution parent does not match {:?}",
                height,
                hash,
                parent_hash
            );
        }

        let is_head = fork_choice > fork_key_of(&inner.head);
        let reorg = is_head && !parent.ptr_eq(&inner.head);
        if linear && (!is_head || reorg) {
            return errf!("fast-sync block is not a linear head extension");
        }
        let scheduled_height = height_of(&inner.scheduled_root);
        let roll = if is_head && height > scheduled_height.saturating_add(unstable_window) {
            Some(inner.plan_roll_from(&chunk, scheduled_height + 1)?)
        } else {
            None
        };

        // No fallible tree computation remains after the chunk is frozen and
        // published through the parent's strong children list.
        parent.attach_block_child(&chunk)?;
        let tx_diff = is_head
            .then(|| canonical_tx_diff(&inner.head, &chunk))
            .unwrap_or_default();
        if is_head {
            inner.head = chunk.clone();
            inner.epoch = inner.epoch.wrapping_add(1);
            if let Some(job) = &roll {
                inner.scheduled_root = job.new_root.clone();
            }
        }
        Ok(Inserted {
            is_head,
            reorg,
            roll,
            confirmed_txs: tx_diff.confirmed,
            reverted_txs: tx_diff.reverted,
        })
    }

    /// Make `job.new_root` the root and drop everything outside its branch.
    /// Called after the job has been written to disk.
    pub fn validate_roll(&self, job: &RollJob) -> Ret<()> {
        self.lock().validate_roll(job)
    }

    pub fn commit_roll(&self, job: &RollJob) -> Ret<()> {
        let mut inner = self.lock();
        inner.validate_roll(job)?;
        let disk = inner
            .root
            .disk()
            .ok_or_else(|| "durable root is not disk-backed".to_string())?;
        job.new_root.promote_to_root(disk)?;
        inner.root = job.new_root.clone();
        if inner.find(&hash_of(&inner.head)).is_none() {
            inner.head = inner.root.clone();
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn snapshot(&self, hash: &Hash) -> Option<StateChunkRef> {
        let inner = self.lock();
        inner.find(hash)
    }

    /// Construct a block execution chunk from a parent and its matching state
    /// view captured under one tree lock.
    pub fn begin_block_execution(
        &self,
        parent_hash: &Hash,
        block: BlockRef,
        fork_choice: ForkChoiceKey,
    ) -> Ret<Option<(StateChunkRef, StateChunkRef)>> {
        let inner = self.lock();
        let Some(parent) = inner.find(parent_hash) else {
            return Ok(None);
        };
        let height = block.height();
        if height != height_of(&parent) + 1 {
            return errf!(
                "block height {} does not follow parent height {}",
                height,
                height_of(&parent)
            );
        }
        if block.prev_hash() != *parent_hash {
            return errf!(
                "block <{}, {:?}> does not reference execution parent {:?}",
                height,
                block.hash(),
                parent_hash
            );
        }
        let chunk = StateChunkRef::block_exec_on(&parent, block, fork_choice)?;
        Ok(Some((chunk, parent)))
    }

    /// Branch metadata and state captured under one tree lock.
    pub fn snapshot_at(&self, hash: &Hash) -> Option<(StateChunkRef, StateChunkRef, u64)> {
        let inner = self.lock();
        let tip = inner.find(hash)?;
        let height = height_of(&tip);
        Some((tip, inner.root.clone(), height))
    }

    /// `(height, hash)` pairs from the head back `depth` blocks, newest first.
    pub fn back_hashes(&self, depth: u64) -> Vec<(u64, Hash)> {
        let inner = self.lock();
        let mut out = Vec::new();
        let mut cursor = inner.head.clone();
        for _ in 0..depth {
            out.push((height_of(&cursor), hash_of(&cursor)));
            match cursor.parent() {
                Some(parent) => cursor = parent,
                None => break,
            }
        }
        out
    }
}

/// Transaction membership change between two canonical tips. Both paths are
/// inside the locked fork tree; walking their weak parents is therefore safe.
fn canonical_tx_diff(old_head: &StateChunkRef, new_head: &StateChunkRef) -> CanonicalTxDiff {
    let mut old = old_head.clone();
    let mut new = new_head.clone();
    let mut old_branch = Vec::new();
    let mut new_branch = Vec::new();

    while height_of(&old) > height_of(&new) {
        old_branch.push(old.clone());
        old = old
            .parent()
            .expect("old canonical branch must reach the fork point");
    }
    while height_of(&new) > height_of(&old) {
        new_branch.push(new.clone());
        new = new
            .parent()
            .expect("new canonical branch must reach the fork point");
    }
    while !old.ptr_eq(&new) {
        old_branch.push(old.clone());
        new_branch.push(new.clone());
        old = old
            .parent()
            .expect("old canonical branch must reach the common ancestor");
        new = new
            .parent()
            .expect("new canonical branch must reach the common ancestor");
    }

    old_branch.reverse();
    new_branch.reverse();
    let old_txs: Vec<Hash> = old_branch
        .iter()
        .flat_map(StateChunkRef::block_tx_hashes)
        .collect();
    let new_txs: Vec<Hash> = new_branch
        .iter()
        .flat_map(StateChunkRef::block_tx_hashes)
        .collect();
    let old_set: HashSet<Hash> = old_txs.iter().copied().collect();
    let new_set: HashSet<Hash> = new_txs.iter().copied().collect();

    CanonicalTxDiff {
        confirmed: new_txs
            .into_iter()
            .filter(|hash| !old_set.contains(hash))
            .collect(),
        reverted: old_txs
            .into_iter()
            .filter(|hash| !new_set.contains(hash))
            .collect(),
    }
}

impl Inner {
    fn validate_roll(&self, job: &RollJob) -> Ret<()> {
        let current_hash = hash_of(&self.root);
        let current_height = height_of(&self.root);
        if current_hash != job.expected_root_hash || current_height != job.expected_root_height {
            return errf!(
                "root roll out of order: expected <{}, {:?}> but durable root is <{}, {:?}>",
                job.expected_root_height,
                job.expected_root_hash,
                current_height,
                current_hash
            );
        }
        let Some(first) = job.chain.first() else {
            return errf!("root roll has an empty state chain");
        };
        let Some(parent) = first.parent() else {
            return errf!("root roll first chunk has no durable parent");
        };
        if !parent.ptr_eq(&self.root) {
            return errf!("root roll chain does not start above the durable root");
        }
        if !job
            .chain
            .last()
            .is_some_and(|last| last.ptr_eq(&job.new_root))
        {
            return errf!("root roll chain does not end at the new root");
        }
        Ok(())
    }

    /// Depth-first from the root. The tree only ever holds the unstable window,
    /// so this is a handful of nodes; the head is checked first because that is
    /// what every insert looks for.
    fn find(&self, hash: &Hash) -> Option<StateChunkRef> {
        if hash_of(&self.head) == *hash {
            return Some(self.head.clone());
        }
        let mut stack = vec![self.root.clone()];
        while let Some(node) = stack.pop() {
            if hash_of(&node) == *hash {
                return Some(node);
            }
            stack.extend(node.children());
        }
        None
    }

    /// Collect the branch above the last scheduled root. The durable root may
    /// lag behind while the bounded persistence queue is busy.
    fn plan_roll_from(&self, head: &StateChunkRef, new_root_height: u64) -> Ret<RollJob> {
        let old_root_height = height_of(&self.scheduled_root);
        if new_root_height <= old_root_height {
            return errf!(
                "scheduled root must advance above {}, got {}",
                old_root_height,
                new_root_height
            );
        }
        let mut chain = Vec::new();
        let mut cursor = head.clone();
        while height_of(&cursor) > new_root_height {
            cursor = cursor
                .parent()
                .ok_or_else(|| "fork tree branch broken while planning root roll".to_string())?;
        }
        let new_root = cursor.clone();
        while height_of(&cursor) > old_root_height {
            chain.push(cursor.clone());
            cursor = cursor
                .parent()
                .ok_or_else(|| "fork tree branch broken while planning root roll".to_string())?;
        }
        if !cursor.ptr_eq(&self.scheduled_root) {
            return errf!("new head does not descend from the scheduled root");
        }
        chain.reverse();
        Ok(RollJob {
            expected_root_hash: hash_of(&self.scheduled_root),
            expected_root_height: old_root_height,
            new_root,
            chain,
        })
    }
}

fn root_chunk(disk: Arc<dyn DiskDB>, block: BlockRef) -> StateChunkRef {
    StateChunkRef::new_root(disk, block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::{StateLayer, StateRead};
    use std::any::Any;

    struct NoDisk;
    impl DiskDB for NoDisk {
        fn read(&self, _key: &[u8]) -> Option<Vec<u8>> {
            None
        }
        fn save(&self, _key: &[u8], _val: &[u8]) {}
        fn remove(&self, _key: &[u8]) {}
    }

    fn hash(b: u8) -> Hash {
        Hash::from([b; 32])
    }

    #[derive(Debug)]
    struct TestBlock {
        height: u64,
        hash: Hash,
        prev_hash: Hash,
    }

    impl field::Encode for TestBlock {
        fn size(&self) -> usize {
            0
        }

        fn encode_to(&self, _out: &mut Vec<u8>) {}
    }

    impl base::Block for TestBlock {
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
            self.prev_hash
        }

        fn mrklroot(&self) -> Hash {
            Hash::default()
        }

        fn timestamp(&self) -> u64 {
            self.height
        }

        fn transactions(&self) -> &[base::TxRef] {
            &[]
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn block(height: u64, hash: Hash, prev_hash: Hash) -> BlockRef {
        Arc::new(TestBlock {
            height,
            hash,
            prev_hash,
        })
    }

    fn tree() -> Tree {
        Tree::new(Arc::new(NoDisk), block(0, hash(0), Hash::default()))
    }

    fn key(k: u8) -> ForkChoiceKey {
        ForkChoiceKey::new(vec![k])
    }

    #[test]
    fn block_context_retains_the_attached_block() {
        let tree = tree();
        let child = block(1, hash(1), hash(0));
        let (chunk, _) = tree
            .begin_block_execution(&hash(0), child.clone(), key(10))
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&chunk.block_identity().unwrap().block, &child));
        tree.attach(&hash(0), chunk, 4).unwrap();

        let (stored, stored_key) = tree.block_context(&hash(1)).unwrap();
        assert!(Arc::ptr_eq(&stored, &child));
        assert_eq!(stored_key, key(10));
    }

    #[test]
    fn branch_history_follows_the_candidate_parent() {
        let tree = tree();
        attach(&tree, hash(0), hash(1), 1, key(10), None, 4);
        attach(&tree, hash(1), hash(2), 2, key(20), None, 4);
        attach(&tree, hash(1), hash(12), 2, key(15), None, 4);

        let hashes: Vec<_> = tree
            .branch_blocks(&hash(12))
            .unwrap()
            .into_iter()
            .map(|block| block.hash())
            .collect();
        assert_eq!(hashes, vec![hash(0), hash(1), hash(12)]);
    }

    fn attach(
        tree: &Tree,
        parent_hash: Hash,
        child_hash: Hash,
        height: u64,
        fork_choice: ForkChoiceKey,
        write: Option<(&[u8], &[u8])>,
        window: u64,
    ) -> Inserted {
        let (mut chunk, _) = tree
            .begin_block_execution(
                &parent_hash,
                block(height, child_hash, parent_hash),
                fork_choice,
            )
            .unwrap()
            .unwrap();
        if let Some((key, value)) = write {
            chunk.set(key, value.to_vec());
        }
        tree.attach(&parent_hash, chunk, window).unwrap()
    }

    fn attach_txs(
        tree: &Tree,
        parent_hash: Hash,
        child_hash: Hash,
        height: u64,
        fork_choice: ForkChoiceKey,
        txs: &[u8],
    ) -> Inserted {
        let (mut chunk, _) = tree
            .begin_block_execution(
                &parent_hash,
                block(height, child_hash, parent_hash),
                fork_choice,
            )
            .unwrap()
            .unwrap();
        for tx in txs {
            let child = chunk.spawn_tx_child(hash(*tx)).unwrap();
            chunk = child.commit_to_parent().unwrap();
        }
        tree.attach(&parent_hash, chunk, 8).unwrap()
    }

    #[test]
    fn head_follows_fork_choice_not_height() {
        let t = tree();
        attach(&t, hash(0), hash(1), 1, key(10), None, 4);
        attach(&t, hash(0), hash(2), 1, key(20), None, 4);
        // Taller, but on the weaker branch: the head must not follow it.
        attach(&t, hash(1), hash(3), 2, key(15), None, 4);
        assert_eq!(t.head_tip().0, hash(2));
    }

    #[test]
    fn switching_branch_reports_reorg() {
        let t = tree();
        let a = attach(&t, hash(0), hash(1), 1, key(10), None, 4);
        assert!(a.is_head && !a.reorg);
        let b = attach(&t, hash(0), hash(2), 1, key(20), None, 4);
        assert!(b.is_head && b.reorg, "branch switch must be a reorg");
    }

    #[test]
    fn side_branch_does_not_change_canonical_transactions() {
        let t = tree();
        let head = attach_txs(&t, hash(0), hash(1), 1, key(20), &[11]);
        assert_eq!(head.confirmed_txs, vec![hash(11)]);
        assert!(head.reverted_txs.is_empty());

        let side = attach_txs(&t, hash(0), hash(2), 1, key(10), &[22]);
        assert!(!side.is_head);
        assert!(side.confirmed_txs.is_empty());
        assert!(side.reverted_txs.is_empty());
        assert_eq!(t.find(&hash(2)).unwrap().block_tx_hashes(), vec![hash(22)]);
    }

    #[test]
    fn reorg_reports_full_branch_diff_and_excludes_shared_hashes() {
        let t = tree();
        attach_txs(&t, hash(0), hash(1), 1, key(20), &[11, 99]);
        attach_txs(&t, hash(1), hash(2), 2, key(30), &[12]);

        let side = attach_txs(&t, hash(0), hash(3), 1, key(10), &[21, 99]);
        assert!(!side.is_head);
        let switched = attach_txs(&t, hash(3), hash(4), 2, key(40), &[22]);

        assert!(switched.is_head && switched.reorg);
        assert_eq!(switched.confirmed_txs, vec![hash(21), hash(22)]);
        assert_eq!(switched.reverted_txs, vec![hash(11), hash(12)]);
    }

    #[test]
    fn duplicate_insert_is_rejected() {
        let t = tree();
        attach(&t, hash(0), hash(1), 1, key(10), None, 4);
        let (duplicate, _) = t
            .begin_block_execution(&hash(0), block(1, hash(1), hash(0)), key(10))
            .unwrap()
            .unwrap();
        assert!(t.attach(&hash(0), duplicate, 4).is_err());
    }

    #[test]
    fn linear_attach_rejects_before_publishing() {
        let t = tree();
        attach(&t, hash(0), hash(1), 1, key(10), None, 4);

        let (side, _) = t
            .begin_block_execution(&hash(0), block(1, hash(2), hash(0)), key(20))
            .unwrap()
            .unwrap();
        assert!(t.attach_linear(&hash(0), side, 4).is_err());
        assert!(t.find(&hash(2)).is_none());

        let (weak, _) = t
            .begin_block_execution(&hash(1), block(2, hash(3), hash(1)), key(5))
            .unwrap()
            .unwrap();
        assert!(t.attach_linear(&hash(1), weak.clone(), 4).is_err());
        assert!(t.find(&hash(3)).is_none());
        // The failed linear attach did not freeze the chunk; normal fork
        // handling can still attach it as a non-head child.
        assert!(!t.attach(&hash(1), weak, 4).unwrap().is_head);
    }

    #[test]
    fn only_identified_blocks_can_join_the_tree() {
        let t = tree();
        let base = t.snapshot(&hash(0)).unwrap();
        let draft = StateChunkRef::block_draft_on(&base, 1);
        assert!(t.attach(&hash(0), draft, 4).is_err());

        let tx = StateChunkRef::tx_on(&base, hash(1));
        assert!(t.attach(&hash(0), tx, 4).is_err());
    }

    #[test]
    fn snapshot_stacks_the_whole_branch() {
        let t = tree();
        attach(&t, hash(0), hash(1), 1, key(1), Some((b"k", b"v1")), 4);
        attach(&t, hash(1), hash(2), 2, key(2), Some((b"other", b"x")), 4);
        attach(&t, hash(2), hash(3), 3, key(3), Some((b"k", b"v3")), 4);
        // Newest write of `k` wins, and writes from lower blocks are visible.
        let snap = t.snapshot(&hash(3)).unwrap();
        assert_eq!(snap.get(b"k"), Some(b"v3".to_vec()));
        assert_eq!(snap.get(b"other"), Some(b"x".to_vec()));
        // A snapshot of the middle block must not see the later write.
        let mid = t.snapshot(&hash(2)).unwrap();
        assert_eq!(mid.get(b"k"), Some(b"v1".to_vec()));
    }

    #[test]
    fn head_snapshot_epoch_tracks_canonical_head_only() {
        let t = tree();
        attach(&t, hash(0), hash(1), 1, key(10), Some((b"k", b"head-1")), 4);
        let (head_hash, head_height, epoch, snap, _root_pin) = t.head_snapshot();
        assert_eq!((head_hash, head_height), (hash(1), 1));

        attach(&t, hash(0), hash(2), 1, key(5), Some((b"k", b"side")), 4);
        assert_eq!(t.epoch(), epoch, "a side branch must not stale head work");

        attach(&t, hash(0), hash(3), 1, key(20), Some((b"k", b"head-2")), 4);
        assert_ne!(t.epoch(), epoch, "a reorg must stale old head work");
        assert_eq!(snap.get(b"k"), Some(b"head-1".to_vec()));
        let (new_hash, new_height, _, new_snap, _root_pin) = t.head_snapshot();
        assert_eq!((new_hash, new_height), (hash(3), 1));
        assert_eq!(new_snap.get(b"k"), Some(b"head-2".to_vec()));
    }

    #[test]
    fn root_rolls_once_past_the_window() {
        let t = tree();
        for h in 1..=2u8 {
            let r = attach(&t, hash(h - 1), hash(h), h as u64, key(h), None, 2);
            assert!(r.roll.is_none(), "must not roll inside the window");
        }
        let r = attach(&t, hash(2), hash(3), 3, key(3), None, 2);
        let job = r.roll.expect("root must advance past the window");
        assert_eq!(height_of(&job.new_root), 1);
        assert_eq!(job.chain.len(), 1);
        t.commit_roll(&job).unwrap();
        assert_eq!(t.root_height(), 1);
        let root = t.find(&hash(1)).unwrap();
        assert!(root.parent().is_none());
        assert!(root.disk().is_some());
    }

    #[test]
    fn scheduled_roots_can_lead_durable_root_and_commit_in_order() {
        let t = tree();
        for h in 1..=2u8 {
            attach(
                &t,
                hash(h - 1),
                hash(h),
                h as u64,
                key(h),
                Some((&[h], &[h])),
                2,
            );
        }
        let first = attach(&t, hash(2), hash(3), 3, key(3), Some((b"a", b"one")), 2)
            .roll
            .unwrap();
        let second = attach(&t, hash(3), hash(4), 4, key(4), Some((b"b", b"two")), 2)
            .roll
            .unwrap();

        assert_eq!(
            t.root_height(),
            0,
            "planning must not move the durable root"
        );
        assert_eq!(first.expected_root_height, 0);
        assert_eq!(height_of(&first.new_root), 1);
        assert_eq!(second.expected_root_height, 1);
        assert_eq!(height_of(&second.new_root), 2);
        let snap = t.snapshot(&hash(4)).unwrap();
        assert_eq!(snap.get(b"a"), Some(b"one".to_vec()));
        assert_eq!(snap.get(b"b"), Some(b"two".to_vec()));

        assert!(t.validate_roll(&second).is_err());
        assert!(t.commit_roll(&second).is_err());
        assert_eq!(t.root_height(), 0);
        t.commit_roll(&first).unwrap();
        t.commit_roll(&second).unwrap();
        assert_eq!(t.root_height(), 2);
    }

    #[test]
    fn snapshot_survives_a_root_roll() {
        // The bug this design removes: a reader holding a branch view while the
        // root advances and prunes the chunks it walked.
        let t = tree();
        attach(&t, hash(0), hash(1), 1, key(1), Some((b"k", b"v1")), 1);
        let snap = t.snapshot(&hash(1)).unwrap();
        let r = attach(&t, hash(1), hash(2), 2, key(2), Some((b"k", b"v2")), 1);
        t.commit_roll(&r.roll.expect("root should advance"))
            .unwrap();
        // Old chunks are pruned, but the captured view still answers.
        assert_eq!(snap.get(b"k"), Some(b"v1".to_vec()));
    }

    #[test]
    fn block_execution_is_invalid_after_its_parent_branch_is_pruned() {
        let t = tree();
        attach(
            &t,
            hash(0),
            hash(1),
            1,
            key(10),
            Some((b"canonical", b"one")),
            1,
        );
        attach(
            &t,
            hash(0),
            hash(9),
            1,
            key(5),
            Some((b"side", b"value")),
            1,
        );

        let (pending, _) = t
            .begin_block_execution(&hash(9), block(2, hash(8), hash(9)), key(6))
            .unwrap()
            .unwrap();

        let roll = attach(&t, hash(1), hash(2), 2, key(20), None, 1)
            .roll
            .unwrap();
        t.commit_roll(&roll).unwrap();

        assert!(pending.parent().is_none());
        assert!(t.find(&hash(9)).is_none());
    }

    #[test]
    fn root_pin_keeps_a_pruned_optimistic_branch_readable() {
        let t = tree();
        attach(&t, hash(0), hash(1), 1, key(10), None, 1);
        attach(
            &t,
            hash(0),
            hash(9),
            1,
            key(5),
            Some((b"side", b"value")),
            1,
        );
        let (tip, root_pin, _) = t.snapshot_at(&hash(9)).unwrap();

        let roll = attach(&t, hash(1), hash(2), 2, key(20), None, 1)
            .roll
            .unwrap();
        t.commit_roll(&roll).unwrap();

        assert!(!t.contains(&hash(9)));
        assert!(tip.parent().is_some());
        assert_eq!(tip.get(b"side"), Some(b"value".to_vec()));
        drop(root_pin);
        assert!(tip.parent().is_none());
    }

    #[test]
    fn dropped_branch_leaves_the_index() {
        let t = tree();
        attach(&t, hash(0), hash(1), 1, key(1), None, 1);
        attach(&t, hash(0), hash(9), 1, key(0), None, 1);
        let r = attach(&t, hash(1), hash(2), 2, key(2), None, 1);
        t.commit_roll(&r.roll.unwrap()).unwrap();
        // hash(9) was on the losing branch below the new root.
        assert!(t.find(&hash(9)).is_none());
        assert!(t.find(&hash(2)).is_some());
    }
}
