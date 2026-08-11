//! Unified state chunks for block branches and transient execution.
//!
//! Block chunks are frozen and retained by the fork tree. Tx and AST chunks
//! are short-lived overlays: they point weakly at the caller-held parent and
//! merge into it only after successful execution.

use std::sync::{Arc, RwLock, Weak};

use field::Hash;
use sys::{Ret, errf};

use crate::state::{LogEntry, StateLayer, StateRead};
use crate::store::{DiskDB, MemKV};
use crate::{BlockRef, ForkChoiceKey};

#[derive(Clone)]
pub struct BlockIdentity {
    pub hash: Hash,
    pub block: BlockRef,
    pub fork_choice: ForkChoiceKey,
}

impl std::fmt::Debug for BlockIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockIdentity")
            .field("hash", &self.hash)
            .field("height", &self.block.height())
            .field("fork_choice", &self.fork_choice)
            .finish()
    }
}

impl PartialEq for BlockIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.fork_choice == other.fork_choice
    }
}

impl Eq for BlockIdentity {}

#[derive(Clone, Debug)]
pub enum Origin {
    /// `identity == None` is a detached block draft used for packing or state
    /// initialization. Only identified blocks may join the fork tree.
    Block {
        height: u64,
        identity: Option<BlockIdentity>,
    },
    Tx {
        tx_hash: Hash,
    },
    Ast {
        depth: u32,
    },
}

impl Origin {
    pub fn is_block(&self) -> bool {
        matches!(self, Origin::Block { .. })
    }

    pub fn is_exec(&self) -> bool {
        matches!(self, Origin::Tx { .. } | Origin::Ast { .. })
    }

    pub fn block_height(&self) -> Option<u64> {
        match self {
            Origin::Block { height, .. } => Some(*height),
            _ => None,
        }
    }

    pub fn block_identity(&self) -> Option<&BlockIdentity> {
        match self {
            Origin::Block {
                identity: Some(identity),
                ..
            } => Some(identity),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct ChunkDelta {
    pub state: MemKV,
    pub logs: Vec<LogEntry>,
    pub tx_hashes: Vec<Hash>,
}

struct FrozenChunk {
    state: Arc<MemKV>,
    logs: Vec<LogEntry>,
    tx_hashes: Vec<Hash>,
}

impl From<ChunkDelta> for FrozenChunk {
    fn from(delta: ChunkDelta) -> Self {
        Self {
            state: Arc::new(delta.state),
            logs: delta.logs,
            tx_hashes: delta.tx_hashes,
        }
    }
}

enum ChunkBody {
    Writable(ChunkDelta),
    Frozen(FrozenChunk),
    /// The delta was committed, discarded, or extracted from a block draft.
    Consumed,
}

impl Default for ChunkBody {
    fn default() -> Self {
        Self::Writable(ChunkDelta::default())
    }
}

impl ChunkBody {
    fn writable(&self) -> Option<&ChunkDelta> {
        match self {
            Self::Writable(delta) => Some(delta),
            Self::Frozen(_) | Self::Consumed => None,
        }
    }

    fn writable_mut(&mut self) -> Option<&mut ChunkDelta> {
        match self {
            Self::Writable(delta) => Some(delta),
            Self::Frozen(_) | Self::Consumed => None,
        }
    }

    fn take_writable(&mut self) -> Option<ChunkDelta> {
        let current = std::mem::replace(self, Self::Consumed);
        match current {
            Self::Writable(delta) => Some(delta),
            other => {
                *self = other;
                None
            }
        }
    }

    fn freeze(&mut self) -> bool {
        let Some(delta) = self.take_writable() else {
            return false;
        };
        *self = Self::Frozen(delta.into());
        true
    }
}

pub struct StateChunk {
    origin: Origin,
    source: RwLock<Source>,
    body: RwLock<ChunkBody>,
    /// Only attached Block-origin chunks are retained here.
    children: RwLock<Vec<Arc<StateChunk>>>,
}

enum Source {
    Parent(Weak<StateChunk>),
    Disk(Arc<dyn DiskDB>),
}

#[derive(Clone)]
pub struct StateChunkRef(Arc<StateChunk>);

impl std::fmt::Debug for StateChunkRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateChunkRef")
            .field("origin", &self.0.origin)
            .field("ptr", &Arc::as_ptr(&self.0))
            .finish()
    }
}

impl StateChunkRef {
    fn new(origin: Origin, source: Source) -> Self {
        Self::with_body(origin, source, ChunkBody::default())
    }

    fn with_body(origin: Origin, source: Source, body: ChunkBody) -> Self {
        Self(Arc::new(StateChunk {
            origin,
            source: RwLock::new(source),
            body: RwLock::new(body),
            children: RwLock::new(Vec::new()),
        }))
    }

    pub fn new_root(disk: Arc<dyn DiskDB>, block: BlockRef) -> Self {
        let height = block.height();
        Self::with_body(
            Origin::Block {
                height,
                identity: Some(BlockIdentity {
                    hash: block.hash(),
                    block,
                    fork_choice: ForkChoiceKey::from_height(height),
                }),
            },
            Source::Disk(disk),
            ChunkBody::Frozen(ChunkDelta::default().into()),
        )
    }

    pub fn block_draft_on(parent: &Self, height: u64) -> Self {
        Self::new(
            Origin::Block {
                height,
                identity: None,
            },
            Source::Parent(Arc::downgrade(&parent.0)),
        )
    }

    /// Construct the detached genesis draft before a durable root exists.
    pub fn block_draft_on_disk(disk: Arc<dyn DiskDB>, height: u64) -> Self {
        Self::new(
            Origin::Block {
                height,
                identity: None,
            },
            Source::Disk(disk),
        )
    }

    pub fn block_exec_on(parent: &Self, block: BlockRef, fork_choice: ForkChoiceKey) -> Ret<Self> {
        if !parent.origin().is_block() {
            return errf!("block execution parent is not a block chunk");
        }
        let height = block.height();
        Ok(Self::new(
            Origin::Block {
                height,
                identity: Some(BlockIdentity {
                    hash: block.hash(),
                    block,
                    fork_choice,
                }),
            },
            Source::Parent(Arc::downgrade(&parent.0)),
        ))
    }

    pub fn tx_on(parent: &Self, tx_hash: Hash) -> Self {
        Self::new(
            Origin::Tx { tx_hash },
            Source::Parent(Arc::downgrade(&parent.0)),
        )
    }

    pub fn spawn_tx_child(&self, tx_hash: Hash) -> Ret<Self> {
        if !self.origin().is_block() || !self.is_writable() {
            return errf!("transaction child requires a writable block chunk");
        }
        Ok(Self::new(
            Origin::Tx { tx_hash },
            Source::Parent(Arc::downgrade(&self.0)),
        ))
    }

    pub fn spawn_ast_child(&self) -> Ret<Self> {
        let depth = match self.origin() {
            Origin::Tx { .. } => 1,
            Origin::Ast { depth } => depth
                .checked_add(1)
                .ok_or_else(|| sys::Error::fault("AST execution depth overflow"))?,
            Origin::Block { .. } => return errf!("AST child requires a transaction context"),
        };
        if !self.is_writable() {
            return errf!("AST parent chunk is already finalized");
        }
        Ok(Self::new(
            Origin::Ast { depth },
            Source::Parent(Arc::downgrade(&self.0)),
        ))
    }

    pub fn commit_to_parent(self) -> Ret<Self> {
        if !self.origin().is_exec() {
            return errf!("only Tx/Ast chunks may commit to a parent");
        }
        let Some(parent) = self.parent() else {
            return errf!("detached execution chunk has no commit parent");
        };
        match (self.origin(), parent.origin()) {
            (Origin::Tx { .. }, Origin::Block { .. })
            | (Origin::Ast { .. }, Origin::Tx { .. } | Origin::Ast { .. }) => {}
            _ => return errf!("invalid execution chunk parent relationship"),
        }
        // Parent-first is the global lock order for nested execution commits.
        // Holding both body locks makes validation and the delta move atomic
        // with respect to state/log writes and parent finalization.
        let mut parent_body = parent.0.body.write().unwrap();
        let Some(target) = parent_body.writable_mut() else {
            return errf!("execution chunk parent is already finalized");
        };
        let mut child_body = self.0.body.write().unwrap();
        let Some(mut child) = child_body.take_writable() else {
            return errf!("execution chunk already finalized");
        };
        if let Origin::Tx { tx_hash } = self.origin() {
            child.tx_hashes.push(*tx_hash);
        }
        target.state.memry.extend(child.state.memry);
        target.logs.append(&mut child.logs);
        target.tx_hashes.append(&mut child.tx_hashes);
        drop(child_body);
        drop(parent_body);
        Ok(parent)
    }

    pub fn discard(self) -> Ret<()> {
        if !self.origin().is_exec() {
            return errf!("only Tx/Ast chunks may be discarded explicitly");
        }
        if self.0.body.write().unwrap().take_writable().is_none() {
            return errf!("execution chunk already finalized");
        }
        Ok(())
    }

    pub fn origin(&self) -> &Origin {
        &self.0.origin
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn parent(&self) -> Option<Self> {
        let source = self.0.source.read().unwrap();
        match &*source {
            Source::Parent(parent) => parent.upgrade().map(Self),
            Source::Disk(_) => None,
        }
    }

    pub fn block_height(&self) -> Ret<u64> {
        self.origin()
            .block_height()
            .ok_or_else(|| sys::Error::fault("state chunk is not a block"))
    }

    pub fn block_identity(&self) -> Ret<&BlockIdentity> {
        self.origin()
            .block_identity()
            .ok_or_else(|| sys::Error::fault("block chunk has no attachable identity"))
    }

    pub fn validate_tx_identity(&self, expected: &Hash) -> sys::Rerr {
        match self.origin() {
            Origin::Tx { tx_hash } if tx_hash == expected => Ok(()),
            Origin::Tx { tx_hash } => errf!(
                "transaction state chunk hash {:?} does not match transaction {:?}",
                tx_hash,
                expected
            ),
            Origin::Block { .. } | Origin::Ast { .. } => {
                errf!("transaction context requires a Tx state chunk")
            }
        }
    }

    pub fn frozen_state(&self) -> Option<Arc<MemKV>> {
        match &*self.0.body.read().unwrap() {
            ChunkBody::Frozen(frozen) => Some(frozen.state.clone()),
            ChunkBody::Writable(_) | ChunkBody::Consumed => None,
        }
    }

    pub fn block_tx_hashes(&self) -> Vec<Hash> {
        let body = self.0.body.read().unwrap();
        match &*body {
            ChunkBody::Writable(delta) => delta.tx_hashes.clone(),
            ChunkBody::Frozen(frozen) => frozen.tx_hashes.clone(),
            ChunkBody::Consumed => Vec::new(),
        }
    }

    pub fn children(&self) -> Vec<Self> {
        self.0
            .children
            .read()
            .unwrap()
            .iter()
            .cloned()
            .map(Self)
            .collect()
    }

    pub fn attach_block_child(&self, child: &Self) -> Ret<()> {
        if !self.origin().is_block() {
            return errf!("fork-tree parent is not a block chunk");
        }
        let Some(parent) = child.parent() else {
            return errf!("block chunk has no fork-tree parent");
        };
        if !self.ptr_eq(&parent) {
            return errf!("block chunk parent does not match attach target");
        }
        child.block_identity()?;

        let mut body = child.0.body.write().unwrap();
        if !body.freeze() {
            return errf!("block chunk is already attached or finalized");
        }
        drop(body);
        self.0.children.write().unwrap().push(child.0.clone());
        Ok(())
    }

    /// Detach a previously attached child (side-capacity eviction). Returns
    /// whether the child was present. The detached subtree becomes
    /// unreachable and is dropped with its state; the canonical chain never
    /// goes through this path.
    pub fn remove_block_child(&self, child: &Self) -> bool {
        let mut children = self.0.children.write().unwrap();
        let before = children.len();
        children.retain(|c| !Arc::ptr_eq(c, &child.0));
        before != children.len()
    }

    pub fn block_logs(&self) -> Vec<LogEntry> {
        let body = self.0.body.read().unwrap();
        match &*body {
            ChunkBody::Writable(delta) => delta.logs.clone(),
            ChunkBody::Frozen(frozen) => frozen.logs.clone(),
            ChunkBody::Consumed => Vec::new(),
        }
    }

    pub fn emit_log(&self, entry: LogEntry) {
        let mut body = self.0.body.write().unwrap();
        let Some(writable) = body.writable_mut() else {
            drop(body);
            panic!("attempted to mutate finalized state chunk");
        };
        writable.logs.push(entry);
    }

    pub fn take_draft_delta(&self) -> Ret<ChunkDelta> {
        match self.origin() {
            Origin::Block { identity: None, .. } => self
                .0
                .body
                .write()
                .unwrap()
                .take_writable()
                .ok_or_else(|| sys::Error::fault("block draft already finalized")),
            _ => errf!("only a writable block draft can release its delta"),
        }
    }

    pub fn disk(&self) -> Option<Arc<dyn DiskDB>> {
        match &*self.0.source.read().unwrap() {
            Source::Disk(disk) => Some(disk.clone()),
            Source::Parent(_) => None,
        }
    }

    pub fn promote_to_root(&self, disk: Arc<dyn DiskDB>) -> Ret<()> {
        if !self.origin().is_block() || self.frozen_state().is_none() {
            return errf!("only a frozen block chunk can become the durable root");
        }
        let mut source = self.0.source.write().unwrap();
        if !matches!(&*source, Source::Parent(_)) {
            return errf!("state chunk is already disk-backed");
        }
        *source = Source::Disk(disk);
        Ok(())
    }

    fn is_writable(&self) -> bool {
        self.0.body.read().unwrap().writable().is_some()
    }

    fn get_state(&self, key: &[u8]) -> Option<Vec<u8>> {
        {
            let body = self.0.body.read().unwrap();
            match &*body {
                ChunkBody::Writable(delta) => {
                    if let Some(value) = delta.state.get(key) {
                        return value.clone();
                    }
                }
                ChunkBody::Frozen(frozen) => {
                    if let Some(value) = frozen.state.get(key) {
                        return value.clone();
                    }
                }
                ChunkBody::Consumed => {}
            }
        }

        let parent = {
            let source = self.0.source.read().unwrap();
            match &*source {
                Source::Disk(disk) => return crate::read_or_panic(disk.as_ref(), key),
                Source::Parent(parent) => parent
                    .upgrade()
                    .expect("state chunk parent expired while its view was in use"),
            }
        };
        StateChunkRef(parent).get_state(key)
    }
}

impl StateRead for StateChunkRef {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.get_state(key)
    }
}

impl AsRef<dyn StateRead> for StateChunkRef {
    fn as_ref(&self) -> &(dyn StateRead + 'static) {
        self
    }
}

impl StateLayer for StateChunkRef {
    fn set(&mut self, key: &[u8], val: Vec<u8>) {
        let mut body = self.0.body.write().unwrap();
        let Some(writable) = body.writable_mut() else {
            drop(body);
            panic!("attempted to mutate finalized state chunk");
        };
        writable.state.put(key.to_vec(), val);
    }

    fn del(&mut self, key: &[u8]) {
        let mut body = self.0.body.write().unwrap();
        let Some(writable) = body.writable_mut() else {
            drop(body);
            panic!("attempted to mutate finalized state chunk");
        };
        writable.state.del(key.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Barrier};
    use std::thread;

    struct NoDisk;

    impl DiskDB for NoDisk {
        fn read(&self, _key: &[u8]) -> Option<Vec<u8>> {
            None
        }

        fn save(&self, _key: &[u8], _val: &[u8]) {}

        fn remove(&self, _key: &[u8]) {}

        fn try_write(&self, _memkv: &dyn crate::MemDB) -> sys::Rerr {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TestBlock {
        height: u64,
        hash: Hash,
    }

    impl field::Encode for TestBlock {
        fn size(&self) -> usize {
            0
        }

        fn encode_to(&self, _out: &mut Vec<u8>) {}
    }

    impl crate::Block for TestBlock {
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
            self.height
        }

        fn transactions(&self) -> &[crate::TxRef] {
            &[]
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn block(height: u64, hash: Hash) -> BlockRef {
        Arc::new(TestBlock { height, hash })
    }

    fn root() -> StateChunkRef {
        StateChunkRef::new_root(Arc::new(NoDisk), block(0, Hash::default()))
    }

    #[test]
    fn ast_commit_and_discard_are_isolated() {
        let root = root();
        let mut tx = StateChunkRef::tx_on(&root, Hash::default());
        tx.set(b"tx", vec![1]);

        let mut committed = tx.spawn_ast_child().unwrap();
        committed.set(b"ast", vec![2]);
        let tx = committed.commit_to_parent().unwrap();
        assert_eq!(tx.get(b"ast"), Some(vec![2]));

        let mut discarded = tx.spawn_ast_child().unwrap();
        discarded.set(b"ast", vec![3]);
        discarded.discard().unwrap();
        assert_eq!(tx.get(b"ast"), Some(vec![2]));
    }

    #[test]
    fn tx_commit_moves_state_logs_and_hash_into_block() {
        let root = root();
        let mut block = StateChunkRef::block_draft_on(&root, 1);
        let hash = Hash::from([7; 32]);
        let mut tx = block.spawn_tx_child(hash).unwrap();
        tx.set(b"key", vec![9]);
        tx.emit_log(LogEntry {
            topic: "topic".into(),
            data: vec![8],
        });
        block = tx.commit_to_parent().unwrap();

        let delta = block.take_draft_delta().unwrap();
        assert_eq!(delta.state.get(b"key"), Some(&Some(vec![9])));
        assert_eq!(delta.logs.len(), 1);
        assert_eq!(delta.tx_hashes, vec![hash]);
    }

    #[test]
    fn deletion_marker_shadows_parent_state() {
        struct BaseDisk;
        impl DiskDB for BaseDisk {
            fn read(&self, key: &[u8]) -> Option<Vec<u8>> {
                (key == b"key").then(|| vec![1])
            }
            fn save(&self, _key: &[u8], _val: &[u8]) {}
            fn remove(&self, _key: &[u8]) {}
            fn try_write(&self, _memkv: &dyn crate::MemDB) -> sys::Rerr {
                Ok(())
            }
        }

        let root = StateChunkRef::new_root(Arc::new(BaseDisk), block(0, Hash::default()));
        let mut tx = StateChunkRef::tx_on(&root, Hash::default());
        assert_eq!(tx.get(b"key"), Some(vec![1]));
        tx.del(b"key");
        assert_eq!(tx.get(b"key"), None);
    }

    #[test]
    fn execution_chunk_cannot_commit_twice() {
        let root = root();
        let block = StateChunkRef::block_draft_on(&root, 1);
        let tx = block.spawn_tx_child(Hash::default()).unwrap();
        let duplicate = tx.clone();
        tx.commit_to_parent().unwrap();
        assert!(duplicate.commit_to_parent().is_err());
    }

    #[test]
    fn tx_identity_rejects_mismatch_and_non_tx_origins() {
        let expected = Hash::from([1; 32]);
        let root = root();
        let tx = StateChunkRef::tx_on(&root, Hash::from([2; 32]));
        assert!(tx.validate_tx_identity(&expected).is_err());

        let block = StateChunkRef::block_draft_on(&root, 1);
        assert!(block.validate_tx_identity(&expected).is_err());
        let ast = tx.spawn_ast_child().unwrap();
        assert!(ast.validate_tx_identity(&expected).is_err());
    }

    #[test]
    fn discard_clears_speculative_state_from_all_clones() {
        let root = root();
        let mut tx = StateChunkRef::tx_on(&root, Hash::default());
        tx.set(b"speculative", vec![1]);
        let observer = tx.clone();
        tx.discard().unwrap();
        assert_eq!(observer.get(b"speculative"), None);
    }

    #[test]
    fn expired_parent_is_not_reported_as_a_missing_state_key() {
        let tx = {
            let root = root();
            StateChunkRef::tx_on(&root, Hash::default())
        };
        assert!(tx.parent().is_none());
        assert!(catch_unwind(AssertUnwindSafe(|| tx.get(b"key"))).is_err());
    }

    #[test]
    fn attached_block_freezes_state_logs_and_tx_hashes_together() {
        let root = root();
        let block_hash = Hash::from([1; 32]);
        let tx_hash = Hash::from([2; 32]);
        let mut block = StateChunkRef::block_exec_on(
            &root,
            block(1, block_hash),
            ForkChoiceKey::from_height(1),
        )
        .unwrap();
        let mut tx = block.spawn_tx_child(tx_hash).unwrap();
        tx.set(b"key", vec![9]);
        tx.emit_log(LogEntry {
            topic: "topic".into(),
            data: vec![8],
        });
        block = tx.commit_to_parent().unwrap();

        root.attach_block_child(&block).unwrap();
        let body = block.0.body.read().unwrap();
        assert!(matches!(&*body, ChunkBody::Frozen(_)));
        drop(body);
        assert_eq!(block.get(b"key"), Some(vec![9]));
        assert_eq!(block.block_logs().len(), 1);
        assert_eq!(block.block_tx_hashes(), vec![tx_hash]);
    }

    #[test]
    fn reads_survive_parent_to_disk_source_promotion() {
        struct BaseDisk;
        impl DiskDB for BaseDisk {
            fn read(&self, key: &[u8]) -> Option<Vec<u8>> {
                (key == b"base").then(|| vec![1])
            }
            fn save(&self, _key: &[u8], _val: &[u8]) {}
            fn remove(&self, _key: &[u8]) {}
            fn try_write(&self, _memkv: &dyn crate::MemDB) -> sys::Rerr {
                Ok(())
            }
        }

        let disk = Arc::new(BaseDisk);
        let root = StateChunkRef::new_root(disk.clone(), block(0, Hash::default()));
        let first = StateChunkRef::block_exec_on(
            &root,
            block(1, Hash::from([1; 32])),
            ForkChoiceKey::from_height(1),
        )
        .unwrap();
        root.attach_block_child(&first).unwrap();
        let second = StateChunkRef::block_exec_on(
            &first,
            block(2, Hash::from([2; 32])),
            ForkChoiceKey::from_height(2),
        )
        .unwrap();
        first.attach_block_child(&second).unwrap();

        let reader = second.clone();
        let barrier = Arc::new(Barrier::new(2));
        let read_barrier = barrier.clone();
        let reads = thread::spawn(move || {
            read_barrier.wait();
            for _ in 0..10_000 {
                assert_eq!(reader.get(b"base"), Some(vec![1]));
            }
        });
        barrier.wait();
        first.promote_to_root(disk).unwrap();
        drop(root);
        reads.join().unwrap();
        assert!(first.parent().is_none());
    }

    #[test]
    fn write_racing_commit_is_either_included_or_rejected() {
        let root = root();
        let block = StateChunkRef::block_draft_on(&root, 1);
        let tx = block.spawn_tx_child(Hash::default()).unwrap();
        let commit_tx = tx.clone();
        let body_guard = tx.0.body.write().unwrap();
        let barrier = Arc::new(Barrier::new(3));

        let mut writer = tx.clone();
        let writer_barrier = barrier.clone();
        let writer = thread::spawn(move || {
            writer_barrier.wait();
            catch_unwind(AssertUnwindSafe(|| writer.set(b"race", vec![1]))).is_ok()
        });
        let commit_barrier = barrier.clone();
        let commit = thread::spawn(move || {
            commit_barrier.wait();
            commit_tx.commit_to_parent()
        });
        barrier.wait();
        drop(body_guard);

        let write_succeeded = writer.join().unwrap();
        commit.join().unwrap().unwrap();
        assert_eq!(block.get(b"race").is_some(), write_succeeded);
    }

    #[test]
    fn write_racing_attach_is_either_frozen_or_rejected() {
        let root = root();
        let child = StateChunkRef::block_exec_on(
            &root,
            block(1, Hash::from([1; 32])),
            ForkChoiceKey::from_height(1),
        )
        .unwrap();
        let body_guard = child.0.body.write().unwrap();
        let barrier = Arc::new(Barrier::new(3));

        let mut writer = child.clone();
        let writer_barrier = barrier.clone();
        let writer = thread::spawn(move || {
            writer_barrier.wait();
            catch_unwind(AssertUnwindSafe(|| writer.set(b"race", vec![1]))).is_ok()
        });
        let attach_child = child.clone();
        let attach_root = root.clone();
        let attach_barrier = barrier.clone();
        let attach = thread::spawn(move || {
            attach_barrier.wait();
            attach_root.attach_block_child(&attach_child)
        });
        barrier.wait();
        drop(body_guard);

        let write_succeeded = writer.join().unwrap();
        attach.join().unwrap().unwrap();
        assert_eq!(child.get(b"race").is_some(), write_succeeded);
        assert!(child.frozen_state().is_some());
    }
}
