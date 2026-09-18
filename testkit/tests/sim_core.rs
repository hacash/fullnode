use std::sync::Arc;

use base::{Context, DiskDB, LogBackend, LogEntry, StateLayer, StateRead, VmEntry};
use field::Address;
use testkit::sim::{
    context::TestContext, disk::VerifiedMemDiskDB, dualchain::DualMemChain, logs::MemLogs,
    memchain::MemChain, state::FlatMemState, vm::new_counter_vm,
};

#[test]
fn verified_disk_applies_and_restores_atomic_batches() {
    let disk = VerifiedMemDiskDB::new();
    disk.save(b"one", b"1");
    let checkpoint = disk.snapshot();

    let mut batch = base::MemKV::new();
    batch.put(b"two".to_vec(), b"2".to_vec());
    batch.del(b"one".to_vec());
    disk.try_write(&batch).expect("apply batch");
    assert_eq!(disk.read(b"one").unwrap(), None);
    assert_eq!(disk.read(b"two").unwrap(), Some(b"2".to_vec()));

    disk.restore(&checkpoint);
    assert_eq!(disk.entries(), vec![(b"one".to_vec(), b"1".to_vec())]);
}

#[test]
fn flat_state_snapshots_are_isolated() {
    let mut state = FlatMemState::new();
    state.set(b"answer", b"42".to_vec());
    let snapshot = state.snapshot();
    state.set(b"answer", b"43".to_vec());
    state.del(b"answer");
    assert_eq!(state.get(b"answer").unwrap(), None);
    state.restore(snapshot);
    assert_eq!(state.get(b"answer").unwrap(), Some(b"42".to_vec()));
}

#[test]
fn memory_logs_obey_block_boundaries() {
    let logs = MemLogs::new();
    logs.append_block_logs(
        7,
        &[LogEntry {
            topic: "event".into(),
            data: vec![1],
        }],
    )
    .unwrap();
    assert_eq!(logs.len_at(7), 1);
    assert_eq!(logs.load_block_logs(7).unwrap()[0].data, vec![1]);
    logs.remove_block_logs(7).unwrap();
    assert!(logs.load_block_logs(7).unwrap().is_empty());
}

#[test]
fn context_preserves_vm_ownership_and_records_logs() {
    let services = Arc::new(app::standard_registry().expect("standard services"));
    let mut context = TestContext::with_stub_tx(services);
    let (vm, counter) = new_counter_vm();
    context.set_vm(vm);
    context.emit_log(LogEntry {
        topic: "test".into(),
        data: vec![7],
    });
    context
        .vm_call(VmEntry::Raw(Box::new(())))
        .expect("mock VM call");
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(context.take_logs()[0].topic, "test");
    assert_eq!(context.tx().main(), Address::default());
}

#[test]
fn memchain_tracks_transaction_receipts_across_blocks() {
    let mut chain = MemChain::new().expect("memchain");
    let hash = chain.submit(Arc::new(testkit::sim::tx::DummyTx));
    let block = chain.mine_block();
    assert_eq!(block.height, 1);
    block.receipt(&hash).expect("receipt").expect_success();
    assert_eq!(chain.height(), 1);
    assert_eq!(chain.pending_len(), 0);
}

#[test]
fn dual_memchain_detects_no_divergence_for_identical_transactions() {
    let mut chain = DualMemChain::new().expect("dual memchain");
    chain.submit(Arc::new(testkit::sim::tx::DummyTx));
    chain.mine_block().expect_all_success();
    chain.assert_same();
}
