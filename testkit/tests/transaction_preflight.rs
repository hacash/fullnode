use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base::{
    Block, BlockRef, CoreState, CoreStateRead, DiskDB, Env, ExecutionServices, StateChunkRef, TransactionSign, TxRef,
};
use field::{Address, Amount, AssetAmt, AssetSmelt, Balance, BlockHeight, BytesW1, DiamondName, DiamondNumber, DiamondNumberAuto, DiamondOwnedForm, DiamondSto, Encode, Fold64, Hash, Uint1};
use mint::api::preflight::execute_signed_transaction;
use protocol::{
    action_std::{TexCell, TexCellExecute, TransferAssetTo, TransferHacTo},
    tx_std::StdTransaction,
};
use sys::Account;

struct MemoryDisk;
impl DiskDB for MemoryDisk {
    fn read(&self, _key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
        Ok(None)
    }
    fn save(&self, _key: &[u8], _value: &[u8]) {}
    fn remove(&self, _key: &[u8]) {}
    fn try_write(&self, _values: &dyn base::MemDB) -> sys::Rerr {
        Ok(())
    }
}

#[derive(Debug)]
struct TestBlock;
impl Encode for TestBlock {
    fn size(&self) -> usize {
        0
    }
    fn encode_to(&self, _out: &mut Vec<u8>) {}
}
impl Block for TestBlock {
    fn version(&self) -> u8 {
        1
    }
    fn height(&self) -> u64 {
        0
    }
    fn hash(&self) -> Hash {
        Hash::default()
    }
    fn prev_hash(&self) -> Hash {
        Hash::default()
    }
    fn mrklroot(&self) -> Hash {
        Hash::default()
    }
    fn timestamp(&self) -> u64 {
        0
    }
    fn transactions(&self) -> &[TxRef] {
        &[]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn root() -> StateChunkRef {
    let block: BlockRef = Arc::new(TestBlock);
    StateChunkRef::new_root(Arc::new(MemoryDisk), block)
}

fn signed_transfer(main_key: [u8; 32], signer_key: [u8; 32], to: Address) -> StdTransaction {
    let main = Account::create_by_secret_key_value(main_key).unwrap();
    let signer = Account::create_by_secret_key_value(signer_key).unwrap();
    let mut tx = StdTransaction::new_by(
        hacash_params::TX_TYPE_2,
        Address::from(*main.address()),
        Amount::from("1:244").unwrap(),
        sys::curtimes().saturating_sub(1),
    );
    tx.push_action_in(Arc::new(TransferHacTo::new(
        to,
        Amount::from("1:244").unwrap(),
    )));
    tx.fill_sign_account(&signer).unwrap();
    tx
}

fn execute_and_commit(services: Arc<app::Registry>, state: &StateChunkRef, env: Env, tx: TxRef) -> sys::Rerr {
    tx.verify_signature()?;
    let tx_hash = tx.hash();
    let mut chunk = StateChunkRef::tx_on(state, tx_hash);
    base::init_block_contract_storage_budget(&mut chunk, services.vm_params()?, env.block.height)?;
    let mut execution = services.clone().create_context(env, chunk.clone(), tx.clone())?;
    tx.execute(execution.as_mut())?;
    drop(execution);
    chunk.commit_to_parent()?;
    Ok(())
}

fn env(tx: &dyn TransactionSign) -> Env {
    let mut env = Env::default();
    env.block.height = 1;
    env.tx.ty = tx.ty();
    env.tx.main = tx.main();
    env.tx.addrs = tx.addrs();
    env.tx.fee = tx.fee().clone();
    env
}

fn fixture_action_address(tx: &dyn TransactionSign, index: usize, field: &str) -> Address {
    let json = tx.actions()[index].to_json();
    let marker = format!("\"{field}\":\"");
    let address = json.split_once(&marker).unwrap_or_else(|| panic!("fixture action address field {field} missing in {json}")).1
        .split_once('"').expect("fixture action address terminator").0;
    Address::from_readable(address).unwrap()
}

fn fixture_escrow_address(tx: &dyn TransactionSign) -> Address {
    let index = tx.actions().iter().position(|a| {
        let json = a.to_json();
        json.contains("\"kind\":22") && json.contains("\"height\":126")
    }).expect("fixture contains short-window escrow TEX action");
    fixture_action_address(tx, index, "addr")
}

fn fixture_maker_address(tx: &dyn TransactionSign) -> Address {
    let escrow = fixture_escrow_address(tx);
    for (index, action) in tx.actions().iter().enumerate() {
        if action.to_json().contains("\"kind\":22") {
            let address = fixture_action_address(tx, index, "addr");
            if address != escrow && address != tx.main() { return address; }
        }
    }
    panic!("fixture has no distinct maker TEX signer")
}

fn fixture_fee_address(tx: &dyn TransactionSign) -> Address {
    let marker = "\"to\":\"";
    for action in tx.actions().iter().rev() {
        let json = action.to_json();
        if let Some(address) = json.split_once(marker).and_then(|(_, tail)| tail.split_once('"').map(|(address, _)| address)) {
            return Address::from_readable(address).unwrap();
        }
    }
    panic!("fixture has no ordinary fee transfer")
}

fn signed_tex_transaction(cells: Vec<TexCell>) -> StdTransaction {
    let account = Account::create_by_secret_key_value([37; 32]).unwrap();
    let main = Address::from(*account.address());
    signed_tex_transaction_with_actions(
        &account,
        vec![TexCellExecute::create_signed(main, cells, &account).unwrap()],
    )
}

fn signed_tex_transaction_with_actions(
    account: &Account,
    actions: Vec<TexCellExecute>,
) -> StdTransaction {
    let main = Address::from(*account.address());
    let mut tx = StdTransaction::new_by(
        hacash_params::TX_TYPE_3,
        main,
        Amount::from("1:244").unwrap(),
        sys::curtimes().saturating_sub(1),
    );
    for action in actions {
        tx.push_action_in(Arc::new(action));
    }
    tx.fill_sign_account(&account).unwrap();
    tx
}

#[test]
fn signed_transfer_preflight_executes_fully_but_discards_all_state_writes() {
    let services = app::standard_registry().unwrap();
    let main = Account::create_by_secret_key_value([17; 32]).unwrap();
    let to = Address::from(
        *Account::create_by_secret_key_value([18; 32])
            .unwrap()
            .address(),
    );
    let main_addr = Address::from(*main.address());
    let parent = root();
    let mut state = StateChunkRef::block_draft_on(&parent, 1);
    CoreState::wrap(&mut state)
        .balance_set(&main_addr, &Balance::hac(Amount::from("100:248").unwrap()));
    let before = CoreStateRead::wrap(&state).balance(&main_addr).unwrap();
    let tx: TxRef = Arc::new(signed_transfer([17; 32], [17; 32], to));
    let tx_hash = tx.hash();
    assert_eq!(
        CoreStateRead::wrap(&state).tx_exist(&tx_hash).unwrap(),
        None
    );

    execute_signed_transaction(
        Arc::new(services),
        StateChunkRef::tx_on(&state, tx.hash()),
        env(tx.as_ref()),
        tx.clone(),
        Instant::now() + Duration::from_secs(10),
    )
    .expect("complete transfer execution");

    assert_eq!(
        CoreStateRead::wrap(&state).balance(&main_addr).unwrap(),
        before
    );
    assert_eq!(CoreStateRead::wrap(&state).balance(&to).unwrap(), None);
    assert_eq!(
        CoreStateRead::wrap(&state).tx_exist(&tx_hash).unwrap(),
        None
    );

    assert!(execute_signed_transaction(
        Arc::new(app::standard_registry().unwrap()),
        StateChunkRef::tx_on(&state, tx_hash),
        env(tx.as_ref()),
        tx,
        Instant::now() - Duration::from_secs(1),
    )
    .unwrap_err()
    .to_string()
    .contains("sandbox_wall_clock_timeout"));
    assert_eq!(
        CoreStateRead::wrap(&state).balance(&main_addr).unwrap(),
        before
    );
    assert_eq!(
        CoreStateRead::wrap(&state).tx_exist(&tx_hash).unwrap(),
        None
    );
}

#[test]
fn invalid_signature_and_insufficient_balance_fail_without_state_changes() {
    let services = app::standard_registry().unwrap();
    let main = Account::create_by_secret_key_value([27; 32]).unwrap();
    let main_addr = Address::from(*main.address());
    let to = Address::from(
        *Account::create_by_secret_key_value([28; 32])
            .unwrap()
            .address(),
    );
    let parent = root();
    let mut state = StateChunkRef::block_draft_on(&parent, 1);
    CoreState::wrap(&mut state)
        .balance_set(&main_addr, &Balance::hac(Amount::from("1:244").unwrap()));
    let before = CoreStateRead::wrap(&state).balance(&main_addr).unwrap();

    // The transaction names the funded address but is signed by another key.
    let invalid: TxRef = Arc::new(signed_transfer([27; 32], [29; 32], to));
    let invalid_hash = invalid.hash();
    assert_eq!(
        CoreStateRead::wrap(&state).tx_exist(&invalid_hash).unwrap(),
        None
    );
    assert!(execute_signed_transaction(
        Arc::new(services),
        StateChunkRef::tx_on(&state, invalid.hash()),
        env(invalid.as_ref()),
        invalid,
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap_err()
    .to_string()
    .contains("signature"));
    assert_eq!(
        CoreStateRead::wrap(&state).balance(&main_addr).unwrap(),
        before
    );
    assert_eq!(
        CoreStateRead::wrap(&state).tx_exist(&invalid_hash).unwrap(),
        None
    );

    let services = app::standard_registry().unwrap();
    let insufficient: TxRef = Arc::new(signed_transfer([27; 32], [27; 32], to));
    let insufficient_hash = insufficient.hash();
    assert_eq!(
        CoreStateRead::wrap(&state)
            .tx_exist(&insufficient_hash)
            .unwrap(),
        None
    );
    let insufficient_error = execute_signed_transaction(
        Arc::new(services),
        StateChunkRef::tx_on(&state, insufficient.hash()),
        env(insufficient.as_ref()),
        insufficient,
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap_err()
    .to_string();
    assert!(
        insufficient_error.contains("insufficient"),
        "{insufficient_error}"
    );
    assert_eq!(
        CoreStateRead::wrap(&state).balance(&main_addr).unwrap(),
        before
    );
    assert_eq!(CoreStateRead::wrap(&state).balance(&to).unwrap(), None);
    assert_eq!(
        CoreStateRead::wrap(&state)
            .tx_exist(&insufficient_hash)
            .unwrap(),
        None
    );
}

#[test]
fn signed_tex_preflight_handles_balanced_unbalanced_and_expired_bundles_read_only() {
    let services = app::standard_registry().unwrap();
    let account = Account::create_by_secret_key_value([37; 32]).unwrap();
    let main = Address::from(*account.address());
    let other_account = Account::create_by_secret_key_value([38; 32]).unwrap();
    let other = Address::from(*other_account.address());
    let parent = root();
    let mut state = StateChunkRef::block_draft_on(&parent, 1);
    CoreState::wrap(&mut state).balance_set(&main, &Balance::hac(Amount::from("100:248").unwrap()));
    let before = CoreStateRead::wrap(&state).balance(&main).unwrap();

    // Separate signed participants exchange value through the same escrow.
    let balanced: TxRef = Arc::new(signed_tex_transaction_with_actions(
        &account,
        vec![
            TexCellExecute::create_signed(
                main,
                vec![TexCell::ZhuPay {
                    haczhu: Fold64::from(100).unwrap(),
                }],
                &account,
            )
            .unwrap(),
            TexCellExecute::create_signed(
                other,
                vec![TexCell::ZhuGet {
                    haczhu: Fold64::from(100).unwrap(),
                }],
                &other_account,
            )
            .unwrap(),
        ],
    ));
    let balanced_hash = balanced.hash();
    assert_eq!(
        CoreStateRead::wrap(&state)
            .tx_exist(&balanced_hash)
            .unwrap(),
        None
    );
    execute_signed_transaction(
        Arc::new(services),
        StateChunkRef::tx_on(&state, balanced.hash()),
        env(balanced.as_ref()),
        balanced,
        Instant::now() + Duration::from_secs(10),
    )
    .expect("balanced TEX bundle executes");
    assert_eq!(CoreStateRead::wrap(&state).balance(&main).unwrap(), before);
    assert_eq!(CoreStateRead::wrap(&state).balance(&other).unwrap(), None);
    assert_eq!(
        CoreStateRead::wrap(&state)
            .tx_exist(&balanced_hash)
            .unwrap(),
        None
    );

    let services = app::standard_registry().unwrap();
    let unbalanced: TxRef = Arc::new(signed_tex_transaction(vec![TexCell::ZhuPay {
        haczhu: Fold64::from(100).unwrap(),
    }]));
    let unbalanced_hash = unbalanced.hash();
    assert_eq!(
        CoreStateRead::wrap(&state)
            .tx_exist(&unbalanced_hash)
            .unwrap(),
        None
    );
    assert!(execute_signed_transaction(
        Arc::new(services),
        StateChunkRef::tx_on(&state, unbalanced.hash()),
        env(unbalanced.as_ref()),
        unbalanced,
        Instant::now() + Duration::from_secs(10),
    )
    .is_err());
    assert_eq!(CoreStateRead::wrap(&state).balance(&main).unwrap(), before);
    assert_eq!(
        CoreStateRead::wrap(&state)
            .tx_exist(&unbalanced_hash)
            .unwrap(),
        None
    );

    let services = app::standard_registry().unwrap();
    let expired: TxRef = Arc::new(signed_tex_transaction(vec![TexCell::CondHeightAtMost {
        height: BlockHeight::from(0),
    }]));
    let expired_hash = expired.hash();
    assert_eq!(
        CoreStateRead::wrap(&state).tx_exist(&expired_hash).unwrap(),
        None
    );
    assert!(execute_signed_transaction(
        Arc::new(services),
        StateChunkRef::tx_on(&state, expired.hash()),
        env(expired.as_ref()),
        expired,
        Instant::now() + Duration::from_secs(10),
    )
    .is_err());
    assert_eq!(CoreStateRead::wrap(&state).balance(&main).unwrap(), before);
    assert_eq!(
        CoreStateRead::wrap(&state).tx_exist(&expired_hash).unwrap(),
        None
    );

    // The outer transaction signature remains valid, while the inner TEX
    // authorization signature is corrupted.
    let mut bad_inner = TexCellExecute::create_signed(
        main,
        vec![TexCell::ZhuPay {
            haczhu: Fold64::from(10).unwrap(),
        }],
        &account,
    )
    .unwrap();
    bad_inner.sign = field::Sign::default();
    let invalid_inner: TxRef = Arc::new(signed_tex_transaction_with_actions(
        &account,
        vec![bad_inner],
    ));
    let invalid_inner_hash = invalid_inner.hash();
    assert_eq!(
        CoreStateRead::wrap(&state)
            .tx_exist(&invalid_inner_hash)
            .unwrap(),
        None
    );
    assert!(execute_signed_transaction(
        Arc::new(app::standard_registry().unwrap()),
        StateChunkRef::tx_on(&state, invalid_inner_hash),
        env(invalid_inner.as_ref()),
        invalid_inner,
        Instant::now() + Duration::from_secs(10),
    )
    .is_err());
    assert_eq!(CoreStateRead::wrap(&state).balance(&main).unwrap(), before);
    assert_eq!(
        CoreStateRead::wrap(&state)
            .tx_exist(&invalid_inner_hash)
            .unwrap(),
        None
    );
}

#[test]
fn repeated_maker_fill_with_escrow_funding_and_platform_fee_executes_atomically() {
    use protocol::action_std::TexCell;

    let services = app::standard_registry().unwrap();
    let taker = Account::create_by_secret_key_value([41; 32]).unwrap();
    let maker = Account::create_by_secret_key_value([42; 32]).unwrap();
    let escrow = Account::create_by_secret_key_value([43; 32]).unwrap();
    let fee = Account::create_by_secret_key_value([44; 32]).unwrap();
    let taker_addr = Address::from(*taker.address());
    let maker_addr = Address::from(*maker.address());
    let escrow_addr = Address::from(*escrow.address());
    let fee_addr = Address::from(*fee.address());
    let parent = root();
    let mut state = StateChunkRef::block_draft_on(&parent, 120);
    let funded = Balance::hac(Amount::from("100:248").unwrap());
    CoreState::wrap(&mut state).balance_set(&taker_addr, &funded);
    CoreState::wrap(&mut state).balance_set(&maker_addr, &funded);
    CoreState::wrap(&mut state).balance_set(&escrow_addr, &funded);
    CoreState::wrap(&mut state).balance_set(&fee_addr, &funded);
    let before_taker = CoreStateRead::wrap(&state).balance(&taker_addr).unwrap();
    let before_maker = CoreStateRead::wrap(&state).balance(&maker_addr).unwrap();
    let before_escrow = CoreStateRead::wrap(&state).balance(&escrow_addr).unwrap();
    let before_fee = CoreStateRead::wrap(&state).balance(&fee_addr).unwrap();

    let guards = |pay_get: Vec<TexCell>| {
        let mut cells = vec![
            TexCell::CondChainIdEq { chainid: field::Uint4::from(0) },
            TexCell::CondHeightAtLeast { height: BlockHeight::from(120) },
            TexCell::CondHeightAtMost { height: BlockHeight::from(126) },
        ];
        cells.extend(pay_get);
        cells
    };
    let e_pay = TexCellExecute::create_signed(escrow_addr, guards(vec![
        TexCell::ZhuPay { haczhu: Fold64::from(400).unwrap() },
    ]), &escrow).unwrap();
    let maker_fill = TexCellExecute::create_signed(maker_addr, guards(vec![
        TexCell::ZhuPay { haczhu: Fold64::from(100).unwrap() },
        TexCell::ZhuGet { haczhu: Fold64::from(200).unwrap() },
    ]), &maker).unwrap();
    let taker_get = TexCellExecute::create_signed(taker_addr, guards(vec![
        TexCell::ZhuGet { haczhu: Fold64::from(200).unwrap() },
    ]), &taker).unwrap();

    let mut tx = StdTransaction::new_by(hacash_params::TX_TYPE_3, taker_addr,
        Amount::from("1:244").unwrap(), sys::curtimes().saturating_sub(1));
    tx.gas_max = Uint1::from(1);
    tx.push_action_in(Arc::new(TransferHacTo::new(escrow_addr, Amount::zhu(400))));
    tx.push_action_in(Arc::new(TransferHacTo::new(fee_addr, Amount::zhu(2))));
    tx.push_action_in(Arc::new(e_pay));
    tx.push_action_in(Arc::new(maker_fill.clone()));
    tx.push_action_in(Arc::new(maker_fill));
    tx.push_action_in(Arc::new(taker_get));
    tx.fill_sign_account(&taker).unwrap();
    let tx: TxRef = Arc::new(tx);
    let mut tx_env = env(tx.as_ref());
    tx_env.block.height = 120;
    execute_signed_transaction(
        Arc::new(services), StateChunkRef::tx_on(&state, tx.hash()), tx_env, tx,
        Instant::now() + Duration::from_secs(10),
    ).expect("two repeated maker fills must clear the TEX ledger");
    assert_eq!(CoreStateRead::wrap(&state).balance(&taker_addr).unwrap(), before_taker);
    assert_eq!(CoreStateRead::wrap(&state).balance(&maker_addr).unwrap(), before_maker);
    assert_eq!(CoreStateRead::wrap(&state).balance(&escrow_addr).unwrap(), before_escrow);
    assert_eq!(CoreStateRead::wrap(&state).balance(&fee_addr).unwrap(), before_fee);
}

#[test]
fn maker_buy_receives_gross_taker_quote_then_fee_and_clears_ledger() {
    let services = app::standard_registry().unwrap();
    let taker = Account::create_by_secret_key_value([51; 32]).unwrap();
    let maker = Account::create_by_secret_key_value([52; 32]).unwrap();
    let escrow = Account::create_by_secret_key_value([53; 32]).unwrap();
    let fee = Account::create_by_secret_key_value([54; 32]).unwrap();
    let taker_addr = Address::from(*taker.address());
    let maker_addr = Address::from(*maker.address());
    let escrow_addr = Address::from(*escrow.address());
    let fee_addr = Address::from(*fee.address());
    let serial = Fold64::from(1028).unwrap();
    let parent = root();
    let mut state = StateChunkRef::block_draft_on(&parent, 120);
    let funded = Balance::hac(Amount::from("100:248").unwrap());
    CoreState::wrap(&mut state).balance_set(&taker_addr, &funded);
    let mut maker_balance = funded.clone();
    maker_balance.asset_set(AssetAmt { serial, amount: Fold64::from(1_000).unwrap() }).unwrap();
    CoreState::wrap(&mut state).balance_set(&maker_addr, &maker_balance);
    CoreState::wrap(&mut state).balance_set(&escrow_addr, &funded);
    CoreState::wrap(&mut state).balance_set(&fee_addr, &funded);
    CoreState::wrap(&mut state).asset_set(&serial, &AssetSmelt {
        serial, supply: Fold64::from(1_000).unwrap(), decimal: Uint1::from(6), issuer: maker_addr,
        ticket: BytesW1::default(), name: BytesW1::default(),
    });
    let before_taker = CoreStateRead::wrap(&state).balance(&taker_addr).unwrap();
    let before_maker = CoreStateRead::wrap(&state).balance(&maker_addr).unwrap();
    let before_escrow = CoreStateRead::wrap(&state).balance(&escrow_addr).unwrap();
    let before_fee = CoreStateRead::wrap(&state).balance(&fee_addr).unwrap();
    let guards = |cells: Vec<TexCell>| {
        let mut out = vec![
            TexCell::CondChainIdEq { chainid: field::Uint4::from(0) },
            TexCell::CondHeightAtLeast { height: BlockHeight::from(120) },
            TexCell::CondHeightAtMost { height: BlockHeight::from(126) },
        ];
        out.extend(cells); out
    };
    let e_pay = TexCellExecute::create_signed(escrow_addr, guards(vec![
        TexCell::ZhuPay { haczhu: Fold64::from(800_000).unwrap() },
    ]), &escrow).unwrap();
    let maker_fill = TexCellExecute::create_signed(maker_addr, guards(vec![
        TexCell::AssetPay { asset: AssetAmt { serial, amount: Fold64::from(10).unwrap() } },
        TexCell::ZhuGet { haczhu: Fold64::from(10_000).unwrap() },
    ]), &maker).unwrap();
    let taker_get = TexCellExecute::create_signed(taker_addr, guards(vec![
        TexCell::AssetGet { asset: AssetAmt { serial, amount: Fold64::from(800).unwrap() } },
    ]), &taker).unwrap();
    let mut tx = StdTransaction::new_by(hacash_params::TX_TYPE_3, taker_addr,
        Amount::from("1:244").unwrap(), sys::curtimes().saturating_sub(1));
    tx.gas_max = Uint1::from(hacash_params::TX_GAS_BUDGET_CAP_BYTE);
    tx.push_action_in(Arc::new(TransferHacTo::new(escrow_addr, Amount::zhu(800_000))));
    tx.push_action_in(Arc::new(e_pay));
    for _ in 0..80 { tx.push_action_in(Arc::new(maker_fill.clone())); }
    tx.push_action_in(Arc::new(taker_get));
    tx.push_action_in(Arc::new(TransferAssetTo::new(fee_addr, AssetAmt { serial, amount: Fold64::from(1).unwrap() })));
    tx.fill_sign_account(&taker).unwrap();
    let tx: TxRef = Arc::new(tx);
    let mut tx_env = env(tx.as_ref()); tx_env.block.height = 120;
    execute_signed_transaction(Arc::new(services), StateChunkRef::tx_on(&state, tx.hash()), tx_env,
        tx, Instant::now() + Duration::from_secs(10)).expect("maker-buy settlement receives gross before normal fee");
    assert_eq!(CoreStateRead::wrap(&state).balance(&taker_addr).unwrap(), before_taker);
    assert_eq!(CoreStateRead::wrap(&state).balance(&maker_addr).unwrap(), before_maker);
    assert_eq!(CoreStateRead::wrap(&state).balance(&escrow_addr).unwrap(), before_escrow);
    assert_eq!(CoreStateRead::wrap(&state).balance(&fee_addr).unwrap(), before_fee);
}

#[test]
fn builder_generated_80_lot_hac_asset_settlement_executes_fully() {
    let services = Arc::new(app::standard_registry().unwrap());
    let raw = hex::decode(include_str!("fixtures/tex-settlement-builder-80.hex").trim()).unwrap();
    let (tx, used) = protocol::tx_std::create_transaction_type3(services.as_ref(), &raw).unwrap();
    assert_eq!(used, raw.len());
    assert_eq!(tx.ty(), hacash_params::TX_TYPE_3);

    let taker = Account::create_by_password("settlement taker").unwrap();
    let maker = Account::create_by_password("settlement maker").unwrap();
    let escrow = Account::create_by_password("settlement escrow").unwrap();
    let fee = Account::create_by_password("settlement fee").unwrap();
    let taker_addr = Address::from(*taker.address());
    let maker_addr = Address::from(*maker.address());
    let escrow_addr = Address::from(*escrow.address());
    let fee_addr = Address::from(*fee.address());
    let serial = Fold64::from(1028).unwrap();
    let parent = root();
    let mut state = StateChunkRef::block_draft_on(&parent, 120);

    let mut taker_balance = Balance::hac(Amount::from("100:248").unwrap());
    taker_balance.asset_set(AssetAmt { serial, amount: Fold64::from(2_000_000_000).unwrap() }).unwrap();
    CoreState::wrap(&mut state).balance_set(&taker_addr, &taker_balance);
    CoreState::wrap(&mut state).balance_set(&maker_addr, &Balance::hac(Amount::zhu(1_000_000_000_000)));
    CoreState::wrap(&mut state).balance_set(&escrow_addr, &Balance::default());
    CoreState::wrap(&mut state).balance_set(&fee_addr, &Balance::default());
    CoreState::wrap(&mut state).asset_set(&serial, &AssetSmelt {
        serial, supply: Fold64::from(2_000_000_000).unwrap(), decimal: Uint1::from(6), issuer: maker_addr,
        ticket: BytesW1::default(), name: BytesW1::default(),
    });
    let mut tx_env = env(tx.as_ref());
    tx_env.block.height = 120;
    execute_and_commit(services, &state, tx_env, tx)
        .expect("the actual 80-fill SDK-builder transaction should execute and commit in test state");
    let taker_after = CoreStateRead::wrap(&state).balance(&taker_addr).unwrap().unwrap();
    let maker_after = CoreStateRead::wrap(&state).balance(&maker_addr).unwrap().unwrap();
    let fee_after = CoreStateRead::wrap(&state).balance(&fee_addr).unwrap().unwrap();
    let escrow_after = CoreStateRead::wrap(&state).balance(&escrow_addr).unwrap().unwrap();
    assert_eq!(taker_after.asset(serial).unwrap().amount.uint(), 395_200_000);
    assert_eq!(maker_after.asset(serial).unwrap().amount.uint(), 1_600_000_000);
    assert_eq!(fee_after.asset(serial).unwrap().amount.uint(), 4_800_000);
    assert_eq!(escrow_after.asset(serial).map(|asset| asset.amount.uint()).unwrap_or(0), 0);
}


#[test]
fn builder_generated_max_200_hacd_group_executes_fully() {
    let services = Arc::new(app::standard_registry().unwrap());
    let raw = hex::decode(include_str!("fixtures/tex-settlement-builder-hacd-200.hex").trim()).unwrap();
    let (tx, used) = protocol::tx_std::create_transaction_type3(services.as_ref(), &raw).unwrap();
    assert_eq!(used, raw.len());
    assert_eq!(tx.ty(), hacash_params::TX_TYPE_3);

    let taker = Account::create_by_password("settlement HACD taker").unwrap();
    let maker = Account::create_by_password("settlement HACD maker").unwrap();
    let escrow = Account::create_by_password("settlement HACD escrow").unwrap();
    let fee = Account::create_by_password("settlement HACD fee").unwrap();
    let taker_addr = Address::from(*taker.address());
    let maker_addr = Address::from(*maker.address());
    let escrow_addr = Address::from(*escrow.address());
    let fee_addr = Address::from(*fee.address());
    let parent = root();
    let mut state = StateChunkRef::block_draft_on(&parent, 120);
    let mut taker_balance = Balance::hac(Amount::from("100:248").unwrap());
    taker_balance.diamond = DiamondNumberAuto::from_diamond(&DiamondNumber::from(200));
    CoreState::wrap(&mut state).balance_set(&taker_addr, &taker_balance);
    CoreState::wrap(&mut state).balance_set(&maker_addr, &Balance::hac(Amount::from("100:248").unwrap()));
    CoreState::wrap(&mut state).balance_set(&escrow_addr, &Balance::default());
    CoreState::wrap(&mut state).balance_set(&fee_addr, &Balance::default());
    let alphabet = b"WTYUIAHXVMEKBSZN";
    let mut owned = DiamondOwnedForm::default();
    let mut seeded_names = Vec::with_capacity(200);
    for index in 0..200usize {
        let mut n = index;
        let mut chars = [alphabet[0]; 6];
        for ch in chars.iter_mut().rev() {
            *ch = alphabet[n % alphabet.len()];
            n /= alphabet.len();
        }
        let name = DiamondName::from_readable(String::from_utf8(chars.to_vec()).unwrap()).unwrap();
        seeded_names.push(name);
        CoreState::wrap(&mut state).diamond_set(&name, &DiamondSto {
            status: Uint1::from(1), address: taker_addr,
            prev_engraved_height: BlockHeight::from(0), inscripts: field::Inscripts::default(),
        });
        owned.push_one(&name).unwrap();
    }
    CoreState::wrap(&mut state).diamond_owned_set(&taker_addr, &owned);

    let mut tx_env = env(tx.as_ref());
    tx_env.block.height = 120;
    let actions = tx.actions();
    assert_eq!(actions.len(), 44);
    let input = actions[0].as_any().downcast_ref::<protocol::action_std::TransferHacdTo>().unwrap();
    assert_eq!(input.diamonds.length(), 200);
    assert!(actions.last().unwrap().description().contains("0.0012"), "fee action: {}", actions.last().unwrap().description());
    assert!(actions[42].description().contains("get 0.4 HAC"), "receive action: {}", actions[42].description());
    execute_and_commit(services, &state, tx_env, tx)
        .expect("actual 200-name SDK-builder transaction should execute and commit in test state");
    let taker_after = CoreStateRead::wrap(&state).balance(&taker_addr).unwrap().unwrap();
    let maker_after = CoreStateRead::wrap(&state).balance(&maker_addr).unwrap().unwrap();
    assert_eq!(taker_after.diamond.to_diamond().unwrap().uint(), 0);
    assert_eq!(maker_after.diamond.to_diamond().unwrap().uint(), 200);
    for name in seeded_names {
        assert_eq!(CoreStateRead::wrap(&state).diamond(&name).unwrap().unwrap().address, maker_addr);
    }
    assert!(CoreStateRead::wrap(&state).diamond_owned(&escrow_addr).unwrap().is_none());
    assert_eq!(CoreStateRead::wrap(&state).balance(&fee_addr).unwrap().unwrap().hacash.to_zhu_u64().unwrap(), 120_000);
    assert_eq!(maker_after.hacash.to_zhu_u64().unwrap(), 9_960_000_000);
}

#[test]
fn builder_generated_80_lot_asset_asset_settlement_executes_and_accounts_for_fee() {
    let services = Arc::new(app::standard_registry().unwrap());
    let raw = hex::decode(include_str!("fixtures/tex-settlement-builder-asset-asset.hex").trim()).unwrap();
    let (tx, used) = protocol::tx_std::create_transaction_type3(services.as_ref(), &raw).unwrap();
    assert_eq!(used, raw.len());
    let taker = Account::create_by_password("settlement AA taker").unwrap();
    let maker = Account::create_by_password("settlement AA maker").unwrap();
    let escrow = Account::create_by_password("settlement AA escrow").unwrap();
    let fee = Account::create_by_password("settlement AA fee").unwrap();
    let taker_addr = Address::from(*taker.address());
    let maker_addr = Address::from(*maker.address());
    let escrow_addr = Address::from(*escrow.address());
    let fee_addr = Address::from(*fee.address());
    let base_serial = Fold64::from(1028).unwrap();
    let quote_serial = Fold64::from(1029).unwrap();
    let parent = root();
    let mut state = StateChunkRef::block_draft_on(&parent, 120);
    let mut taker_balance = Balance::hac(Amount::from("100:248").unwrap());
    taker_balance.asset_set(AssetAmt { serial: quote_serial, amount: Fold64::from(2_000).unwrap() }).unwrap();
    let mut maker_balance = Balance::hac(Amount::from("100:248").unwrap());
    maker_balance.asset_set(AssetAmt { serial: base_serial, amount: Fold64::from(1_000).unwrap() }).unwrap();
    CoreState::wrap(&mut state).balance_set(&taker_addr, &taker_balance);
    CoreState::wrap(&mut state).balance_set(&maker_addr, &maker_balance);
    CoreState::wrap(&mut state).balance_set(&escrow_addr, &Balance::default());
    CoreState::wrap(&mut state).balance_set(&fee_addr, &Balance::default());
    for (serial, issuer) in [(base_serial, maker_addr), (quote_serial, taker_addr)] {
        CoreState::wrap(&mut state).asset_set(&serial, &AssetSmelt {
            serial, supply: Fold64::from(100_000).unwrap(), decimal: Uint1::from(6), issuer,
            ticket: BytesW1::default(), name: BytesW1::default(),
        });
    }
    let mut tx_env = env(tx.as_ref()); tx_env.block.height = 120;
    execute_and_commit(services, &state, tx_env, tx)
        .expect("actual 80-fill Asset/Asset builder transaction should execute");
    let taker_after = CoreStateRead::wrap(&state).balance(&taker_addr).unwrap().unwrap();
    let maker_after = CoreStateRead::wrap(&state).balance(&maker_addr).unwrap().unwrap();
    let fee_after = CoreStateRead::wrap(&state).balance(&fee_addr).unwrap().unwrap();
    let escrow_after = CoreStateRead::wrap(&state).balance(&escrow_addr).unwrap().unwrap();
    assert_eq!(taker_after.asset(base_serial).unwrap().amount.uint(), 800);
    assert_eq!(taker_after.asset(quote_serial).unwrap().amount.uint(), 395);
    assert_eq!(maker_after.asset(base_serial).unwrap().amount.uint(), 200);
    assert_eq!(maker_after.asset(quote_serial).unwrap().amount.uint(), 1_600);
    assert_eq!(fee_after.asset(quote_serial).unwrap().amount.uint(), 5);
    assert_eq!(escrow_after.asset(quote_serial).map(|a| a.amount.uint()).unwrap_or(0), 0);
}

#[test]
fn builder_generated_maker_sell_hacd_transfers_named_diamond_and_fee() {
    let services = Arc::new(app::standard_registry().unwrap());
    let raw = hex::decode(include_str!("fixtures/tex-settlement-builder-hacd-sell.hex").trim()).unwrap();
    let (tx, used) = protocol::tx_std::create_transaction_type3(services.as_ref(), &raw).unwrap();
    assert_eq!(used, raw.len());
    let taker = Account::create_by_password("settlement HACD sell taker").unwrap();
    let maker = Account::create_by_password("settlement HACD sell maker").unwrap();
    let escrow = Account::create_by_password("settlement HACD sell escrow").unwrap();
    let fee = Account::create_by_password("settlement HACD sell fee").unwrap();
    let taker_addr = Address::from(*taker.address());
    let maker_addr = Address::from(*maker.address());
    let escrow_addr = Address::from(*escrow.address());
    let fee_addr = Address::from(*fee.address());
    let name = DiamondName::from_readable("WTYUIA").unwrap();
    let parent = root();
    let mut state = StateChunkRef::block_draft_on(&parent, 120);
    CoreState::wrap(&mut state).balance_set(&taker_addr, &Balance::hac(Amount::from("100:248").unwrap()));
    let mut maker_balance = Balance::hac(Amount::from("100:248").unwrap());
    maker_balance.diamond = DiamondNumberAuto::from_diamond(&DiamondNumber::from(1));
    CoreState::wrap(&mut state).balance_set(&maker_addr, &maker_balance);
    CoreState::wrap(&mut state).balance_set(&escrow_addr, &Balance::default());
    CoreState::wrap(&mut state).balance_set(&fee_addr, &Balance::default());
    CoreState::wrap(&mut state).diamond_set(&name, &DiamondSto {
        status: Uint1::from(1), address: maker_addr,
        prev_engraved_height: BlockHeight::from(0), inscripts: field::Inscripts::default(),
    });
    let mut owned = DiamondOwnedForm::default(); owned.push_one(&name).unwrap();
    CoreState::wrap(&mut state).diamond_owned_set(&maker_addr, &owned);
    let mut tx_env = env(tx.as_ref()); tx_env.block.height = 120;
    let actions = tx.actions();
    assert_eq!(actions.len(), 5);
    assert!(actions[1].description().contains("0.00003"), "fee: {}", actions[1].description());
    execute_and_commit(services, &state, tx_env, tx)
        .expect("actual maker-sells-HACD builder transaction should execute");
    let taker_after = CoreStateRead::wrap(&state).balance(&taker_addr).unwrap().unwrap();
    let maker_after = CoreStateRead::wrap(&state).balance(&maker_addr).unwrap().unwrap();
    let fee_after = CoreStateRead::wrap(&state).balance(&fee_addr).unwrap().unwrap();
    assert_eq!(taker_after.diamond.to_diamond().unwrap().uint(), 1);
    assert_eq!(maker_after.diamond.to_diamond().unwrap().uint(), 0);
    assert_eq!(CoreStateRead::wrap(&state).diamond(&name).unwrap().unwrap().address, taker_addr);
    assert_eq!(fee_after.hacash.to_zhu_u64().unwrap(), 3_000);
}

#[test]
fn builder_generated_reverse_direction_matrix_executes_and_accounts_for_each_leg() {
    let services = Arc::new(app::standard_registry().unwrap());
    let base_serial = Fold64::from(1028).unwrap();
    let quote_serial = Fold64::from(1029).unwrap();

    // HAC/Asset: maker buys HAC, so the taker pays HAC and receives gross
    // quote assets before paying the one-unit platform fee.
    {
        let raw = hex::decode(include_str!("fixtures/tex-settlement-builder-reverse-ha.hex").trim()).unwrap();
        let (tx, used) = protocol::tx_std::create_transaction_type3(services.as_ref(), &raw).unwrap();
        assert_eq!(used, raw.len());
        let (ta, ma, ea, fa) = (tx.main(), fixture_maker_address(tx.as_ref()), fixture_escrow_address(tx.as_ref()), fixture_fee_address(tx.as_ref()));
        let parent = root(); let mut state = StateChunkRef::block_draft_on(&parent, 120);
        let mut tb = Balance::hac(Amount::zhu(10_000_000_000_000));
        tb.asset_set(AssetAmt { serial: base_serial, amount: Fold64::from(100).unwrap() }).unwrap();
        let mut mb = Balance::hac(Amount::zhu(10_000_000_000_000));
        mb.asset_set(AssetAmt { serial: base_serial, amount: Fold64::from(100).unwrap() }).unwrap();
        CoreState::wrap(&mut state).balance_set(&ta, &tb); CoreState::wrap(&mut state).balance_set(&ma, &mb);
        CoreState::wrap(&mut state).balance_set(&ea, &Balance::hac(Amount::zhu(10_000_000_000_000))); CoreState::wrap(&mut state).balance_set(&fa, &Balance::hac(Amount::zhu(10_000_000_000_000)));
        for (serial, issuer) in [(base_serial, ma), (quote_serial, ta)] {
            CoreState::wrap(&mut state).asset_set(&serial, &AssetSmelt { serial, supply: Fold64::from(1_000).unwrap(), decimal: Uint1::from(6), issuer, ticket: BytesW1::default(), name: BytesW1::default() });
        }
        CoreState::wrap(&mut state).asset_set(&base_serial, &AssetSmelt { serial: base_serial, supply: Fold64::from(1_000).unwrap(), decimal: Uint1::from(6), issuer: ma, ticket: BytesW1::default(), name: BytesW1::default() });
        let mut tx_env = env(tx.as_ref()); tx_env.block.height = 120;
        execute_and_commit(services.clone(), &state, tx_env, tx).expect("builder-originated HAC/Asset reverse transaction executes");
        let t = CoreStateRead::wrap(&state).balance(&ta).unwrap().unwrap();
        let m = CoreStateRead::wrap(&state).balance(&ma).unwrap().unwrap();
        assert!(t.hacash.to_zhu_u64().unwrap() < 10_000_000_000_000 - 2_000_000);
        assert_eq!(m.hacash.to_zhu_u64().unwrap(), 10_000_000_000_000 + 2_000_000);
        assert_eq!(t.asset(base_serial).unwrap().amount.uint(), 139);
        assert_eq!(m.asset(base_serial).unwrap().amount.uint(), 60);
        assert_eq!(CoreStateRead::wrap(&state).balance(&fa).unwrap().unwrap().asset(base_serial).unwrap().amount.uint(), 1);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ea).unwrap().unwrap().hacash.to_zhu_u64().unwrap(), 10_000_000_000_000);
    }

    // Asset/Asset: maker buys base asset; fee is charged against the taker's
    // gross quote receipt, leaving the TEX ledger balanced at gross value.
    {
        let raw = hex::decode(include_str!("fixtures/tex-settlement-builder-reverse-aa.hex").trim()).unwrap();
        let (tx, used) = protocol::tx_std::create_transaction_type3(services.as_ref(), &raw).unwrap(); assert_eq!(used, raw.len());
        let (ta, ma, ea, fa) = (tx.main(), fixture_maker_address(tx.as_ref()), fixture_escrow_address(tx.as_ref()), fixture_fee_address(tx.as_ref()));
        let parent = root(); let mut state = StateChunkRef::block_draft_on(&parent, 120);
        let mut tb = Balance::hac(Amount::zhu(10_000_000_000_000)); tb.asset_set(AssetAmt { serial: base_serial, amount: Fold64::from(100).unwrap() }).unwrap();
        tb.asset_set(AssetAmt { serial: quote_serial, amount: Fold64::from(100).unwrap() }).unwrap();
        let mut mb = Balance::hac(Amount::zhu(10_000_000_000_000)); mb.asset_set(AssetAmt { serial: base_serial, amount: Fold64::from(100).unwrap() }).unwrap();
        mb.asset_set(AssetAmt { serial: quote_serial, amount: Fold64::from(100).unwrap() }).unwrap();
        CoreState::wrap(&mut state).balance_set(&ta, &tb); CoreState::wrap(&mut state).balance_set(&ma, &mb);
        CoreState::wrap(&mut state).balance_set(&ea, &Balance::hac(Amount::zhu(10_000_000_000_000))); CoreState::wrap(&mut state).balance_set(&fa, &Balance::hac(Amount::zhu(10_000_000_000_000)));
        for (serial, issuer) in [(base_serial, ma), (quote_serial, ta)] {
            CoreState::wrap(&mut state).asset_set(&serial, &AssetSmelt { serial, supply: Fold64::from(1_000).unwrap(), decimal: Uint1::from(6), issuer, ticket: BytesW1::default(), name: BytesW1::default() });
        }
        let mut tx_env = env(tx.as_ref()); tx_env.block.height = 120;
        execute_and_commit(services.clone(), &state, tx_env, tx).expect("builder-originated Asset/Asset reverse transaction executes");
        let t = CoreStateRead::wrap(&state).balance(&ta).unwrap().unwrap(); let m = CoreStateRead::wrap(&state).balance(&ma).unwrap().unwrap();
        assert_eq!(t.asset(base_serial).unwrap().amount.uint(), 80); assert_eq!(t.asset(quote_serial).unwrap().amount.uint(), 139);
        assert_eq!(m.asset(base_serial).unwrap().amount.uint(), 120); assert_eq!(m.asset(quote_serial).unwrap().amount.uint(), 60);
        assert_eq!(CoreStateRead::wrap(&state).balance(&fa).unwrap().unwrap().asset(quote_serial).unwrap().amount.uint(), 1);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ea).unwrap().unwrap().asset(base_serial), None);
    }

    // HACD/Asset, maker sells its named diamond for 20 asset units.
    {
        let raw = hex::decode(include_str!("fixtures/tex-settlement-builder-hacd-asset-sell.hex").trim()).unwrap();
        let (tx, used) = protocol::tx_std::create_transaction_type3(services.as_ref(), &raw).unwrap(); assert_eq!(used, raw.len());
        let (ta, ma, ea, fa) = (tx.main(), fixture_maker_address(tx.as_ref()), fixture_escrow_address(tx.as_ref()), fixture_fee_address(tx.as_ref()));
        let name = DiamondName::from_readable("WTYUIA").unwrap(); let parent = root(); let mut state = StateChunkRef::block_draft_on(&parent, 120);
        let mut tb = Balance::hac(Amount::zhu(10_000_000_000_000)); tb.asset_set(AssetAmt { serial: base_serial, amount: Fold64::from(100).unwrap() }).unwrap();
        let mut mb = Balance::hac(Amount::zhu(10_000_000_000_000)); mb.diamond = DiamondNumberAuto::from_diamond(&DiamondNumber::from(1));
        CoreState::wrap(&mut state).balance_set(&ta, &tb); CoreState::wrap(&mut state).balance_set(&ma, &mb);
        CoreState::wrap(&mut state).balance_set(&ea, &Balance::hac(Amount::zhu(10_000_000_000_000))); CoreState::wrap(&mut state).balance_set(&fa, &Balance::hac(Amount::zhu(10_000_000_000_000)));
        for (serial, issuer) in [(base_serial, ma)] { CoreState::wrap(&mut state).asset_set(&serial, &AssetSmelt { serial, supply: Fold64::from(100).unwrap(), decimal: Uint1::from(6), issuer, ticket: BytesW1::default(), name: BytesW1::default() }); }
        CoreState::wrap(&mut state).diamond_set(&name, &DiamondSto { status: Uint1::from(1), address: ma, prev_engraved_height: BlockHeight::from(0), inscripts: field::Inscripts::default() });
        let mut owned = DiamondOwnedForm::default(); owned.push_one(&name).unwrap(); CoreState::wrap(&mut state).diamond_owned_set(&ma, &owned);
        let mut tx_env = env(tx.as_ref()); tx_env.block.height = 120;
        execute_and_commit(services.clone(), &state, tx_env, tx).expect("builder-originated HACD/Asset sell executes");
        assert_eq!(CoreStateRead::wrap(&state).diamond(&name).unwrap().unwrap().address, ta);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ta).unwrap().unwrap().diamond.to_diamond().unwrap().uint(), 1);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ma).unwrap().unwrap().asset(base_serial).unwrap().amount.uint(), 20);
        assert_eq!(CoreStateRead::wrap(&state).balance(&fa).unwrap().unwrap().asset(base_serial).unwrap().amount.uint(), 1);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ea).unwrap().unwrap().diamond.to_diamond().unwrap().uint(), 0);
    }

    // HACD/Asset, maker buys the taker's named diamond; taker receives gross
    // assets and separately pays the platform fee.
    {
        let raw = hex::decode(include_str!("fixtures/tex-settlement-builder-hacd-asset-buy.hex").trim()).unwrap();
        let (tx, used) = protocol::tx_std::create_transaction_type3(services.as_ref(), &raw).unwrap(); assert_eq!(used, raw.len());
        let (ta, ma, ea, fa) = (tx.main(), fixture_maker_address(tx.as_ref()), fixture_escrow_address(tx.as_ref()), fixture_fee_address(tx.as_ref()));
        let name = DiamondName::from_readable("WTYUIA").unwrap(); let parent = root(); let mut state = StateChunkRef::block_draft_on(&parent, 120);
        let mut tb = Balance::hac(Amount::zhu(10_000_000_000_000)); tb.asset_set(AssetAmt { serial: base_serial, amount: Fold64::from(100).unwrap() }).unwrap();
        tb.diamond = DiamondNumberAuto::from_diamond(&DiamondNumber::from(1));
        let mut mb = Balance::hac(Amount::zhu(10_000_000_000_000)); mb.asset_set(AssetAmt { serial: base_serial, amount: Fold64::from(100).unwrap() }).unwrap();
        CoreState::wrap(&mut state).balance_set(&ta, &tb); CoreState::wrap(&mut state).balance_set(&ma, &mb);
        CoreState::wrap(&mut state).balance_set(&ea, &Balance::hac(Amount::zhu(10_000_000_000_000))); CoreState::wrap(&mut state).balance_set(&fa, &Balance::hac(Amount::zhu(10_000_000_000_000)));
        CoreState::wrap(&mut state).asset_set(&base_serial, &AssetSmelt { serial: base_serial, supply: Fold64::from(100).unwrap(), decimal: Uint1::from(6), issuer: ma, ticket: BytesW1::default(), name: BytesW1::default() });
        CoreState::wrap(&mut state).diamond_set(&name, &DiamondSto { status: Uint1::from(1), address: ta, prev_engraved_height: BlockHeight::from(0), inscripts: field::Inscripts::default() });
        let mut owned = DiamondOwnedForm::default(); owned.push_one(&name).unwrap(); CoreState::wrap(&mut state).diamond_owned_set(&ta, &owned);
        let maker_json = tx.actions().iter().map(|action| action.to_json())
            .find(|json| json.contains(&format!("\"addr\":\"{}\"", ma.to_readable())))
            .expect("builder transaction includes maker TEX action");
        assert!(maker_json.contains("\"cellid\":7") && maker_json.contains("\"cellid\":6") && !maker_json.contains("\"cellid\":5"),
            "HACD/Asset maker-buy action must pay asset and receive HACD: {maker_json}");
        let mut tx_env = env(tx.as_ref()); tx_env.block.height = 120;
        execute_and_commit(services, &state, tx_env, tx).expect("builder-originated HACD/Asset buy executes");
        assert_eq!(CoreStateRead::wrap(&state).diamond(&name).unwrap().unwrap().address, ma);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ta).unwrap().unwrap().diamond.to_diamond().unwrap().uint(), 0);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ma).unwrap().unwrap().diamond.to_diamond().unwrap().uint(), 1);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ta).unwrap().unwrap().asset(base_serial).unwrap().amount.uint(), 119);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ma).unwrap().unwrap().asset(base_serial).unwrap().amount.uint(), 80);
        assert_eq!(CoreStateRead::wrap(&state).balance(&fa).unwrap().unwrap().asset(base_serial).unwrap().amount.uint(), 1);
        assert_eq!(CoreStateRead::wrap(&state).balance(&ea).unwrap().unwrap().diamond.to_diamond().unwrap().uint(), 0);
    }
}
