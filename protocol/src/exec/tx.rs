//! Transaction execute bodies for prelude + type 1/2/3.

use base::{
    ActionDispatcher, ActionRef, Context, CoreState, ExecFrom, Transaction, TransactionExecute,
    TransactionSign, hac_add, hac_sub,
};
use field::{Amount, Encode, Hash};
use sys::{Rerr, Ret, errf};

use crate::codec::tx::{DefaultPreludeTx, TransactionType1, TransactionType2, TransactionType3};

fn precheck_tx(ctx: &dyn Context, tx: &dyn Transaction, actions: &[ActionRef]) -> Rerr {
    let params = crate::execution_params(ctx.services().as_ref())?;
    if let Some(tx) = tx.as_any().downcast_ref::<TransactionType3>() {
        tx.validate_signer_limit(params.max_type3_signers)?;
    }
    if ctx.env().chain.fast_sync {
        return Ok(());
    }
    if actions.is_empty() {
        return errf!("transaction actions cannot be empty");
    }
    if actions.len() > params.tx_actions_max {
        return errf!(
            "tx actions exceed limit {} > {}",
            actions.len(),
            params.tx_actions_max
        );
    }
    let need = tx.required_flags();
    if let Some(note) =
        crate::facts::activation_finding(tx.ty(), need, ctx.env().chain.consensus_flags)
    {
        return errf!("{}", note);
    }
    crate::level::precheck_tx_actions(
        tx.ty(),
        actions,
        ctx.env().chain.consensus_flags,
        params.ast_tree_depth_max,
        params.tx_actions_max,
    )
}

struct TxExecutePrep {
    block_height: u64,
    tx_hash: Hash,
    main: field::Address,
    fee: Amount,
    has_ast_control: bool,
}

fn prepare_tx_execute(tx: &dyn TransactionSign, ctx: &mut dyn Context) -> Ret<TxExecutePrep> {
    let env = ctx.env();
    let block_height = env.block.height;
    let tx_hash = tx.hash();
    let main = tx.main();
    let fee = tx.fee().clone();
    let has_ast_control = tx.actions().iter().any(|a| a.nested_actions().is_some());
    if !env.chain.fast_sync {
        for note in crate::facts::main_address_findings(main) {
            return errf!("{}", note);
        }
        for note in crate::facts::addr_version_findings(&tx.addrs()) {
            return errf!("{}", note);
        }
        let params = crate::execution_params(ctx.services().as_ref())?;
        if let Some(note) =
            crate::facts::fee_size_finding_with_params(params, fee.size(), block_height)
        {
            return errf!("{}", note);
        }
        if let Some(note) =
            crate::facts::type1_deprecated_finding_with_params(params, tx.ty(), block_height)
        {
            return errf!("{}", note);
        }
        tx.verify_signature()?;
        let existing = {
            let state = CoreState::wrap(ctx.layer());
            state.tx_exist(&tx_hash)
        };
        if let Some(existing) = existing? {
            // Preserve the historical dev exception for the one known duplicate
            // transaction replayed at height 63,448.
            const HISTORICAL_DUPLICATE_TX: [u8; Hash::SIZE] = [
                0xf2, 0x2d, 0xeb, 0x27, 0xdd, 0x28, 0x93, 0x39, 0x7c, 0x2b, 0xc2, 0x03, 0xdd, 0xc9,
                0xbc, 0x90, 0x34, 0xe4, 0x55, 0xfe, 0x63, 0x0d, 0x8e, 0xe3, 0x10, 0xe8, 0xb5, 0xec,
                0xc6, 0xdc, 0x56, 0x28,
            ];
            if existing.uint() != 63_448 || tx_hash != Hash::from(HISTORICAL_DUPLICATE_TX) {
                return errf!(
                    "tx {} already exists in height {}",
                    tx_hash,
                    existing.uint()
                );
            }
        }
    }
    Ok(TxExecutePrep {
        block_height,
        tx_hash,
        main,
        fee,
        has_ast_control,
    })
}

fn mark_tx_exist(ctx: &mut dyn Context, hash: &Hash, height: u64) {
    let mut state = CoreState::wrap(ctx.layer());
    state.tx_exist_set(hash, &field::BlockHeight::from(height));
}

fn record_tx_fee_totals(ctx: &mut dyn Context, tx: &dyn Transaction) -> Rerr {
    let fee_pay = tx.fee_pay().to_238_u64()? as u128;
    let fee_got = tx.fee_got().to_238_u64()? as u128;
    let mut state = CoreState::wrap(ctx.layer());
    base::with_base_total(&mut state, |total| {
        base::total_add_u12(
            &mut total.tx_fee_pay_total_238,
            fee_pay,
            "tx_fee_pay_total_238",
        )?;
        base::total_add_u12(
            &mut total.tx_fee_got_total_238,
            fee_got,
            "tx_fee_got_total_238",
        )
    })
}

fn record_legacy_extra9_burn(ctx: &mut dyn Context, fee: &Amount, fee_got: &Amount) -> Rerr {
    let burn = fee.sub_mode_u128(fee_got)?;
    if !burn.is_positive() {
        return Ok(());
    }
    let mut state = CoreState::wrap(ctx.layer());
    base::with_base_total(&mut state, |total| {
        base::total_add_amount_238(
            &mut total.tx_fee_burn90_238,
            &burn,
            "legacy_tx_extra9_burn_238",
        )
    })
}

fn execute_actions(ctx: &mut dyn Context, actions: &[ActionRef], charge_extra9: bool) -> Rerr {
    for act in actions {
        ctx.exec_from_set(ExecFrom::Top);
        if charge_extra9 {
            let _ = ActionDispatcher::dispatch_top(ctx, act)?;
        } else {
            let _ = ActionDispatcher::dispatch_top_without_extra9(ctx, act)?;
        }
    }
    Ok(())
}

impl TransactionExecute for DefaultPreludeTx {
    fn execute(&self, ctx: &mut dyn Context) -> Rerr {
        hac_add(ctx, &self.address, &self.reward)?;
        Ok(())
    }
}

macro_rules! impl_tx_type_execute {
    ($name:ty) => {
        impl TransactionExecute for $name {
            fn execute(&self, ctx: &mut dyn Context) -> Rerr {
                precheck_tx(ctx, self, &self.actions)?;
                let prep = prepare_tx_execute(self, ctx)?;
                if !ctx.env().chain.fast_sync
                    && <$name>::TYPE != TransactionType3::TYPE
                    && prep.has_ast_control
                {
                    return errf!(
                        "tx type {} cannot include AST control-flow actions; requires at least type 3",
                        <$name>::TYPE
                    );
                }
                if !ctx.env().chain.fast_sync {
                    if let Some(note) = crate::facts::ano_mark_finding(<$name>::TYPE, self.ano_mark[0])
                    {
                        return errf!("{}", note);
                    }
                    if let Some(note) =
                        crate::facts::gas_max_finding_with_params(
                            crate::execution_params(ctx.services().as_ref())?,
                            <$name>::TYPE,
                            self.gas_max.uint(),
                        )
                    {
                        return errf!("{}", note);
                    }
                }

                mark_tx_exist(ctx, &prep.tx_hash, prep.block_height);
                record_tx_fee_totals(ctx, self)?;
                if <$name>::TYPE == TransactionType3::TYPE {
                    let gas_initialized = crate::exec::gas::tx_gas_initialize(ctx)?;
                    execute_actions(ctx, &self.actions, true)?;
                    crate::exec::tex::do_settlement(ctx)?;
                    ctx.run_deferred_phase()?;
                    if gas_initialized {
                        ctx.gas_refund()?;
                    }
                } else {
                    execute_actions(ctx, &self.actions, false)?;
                    crate::exec::tex::do_settlement(ctx)?;
                }
                hac_sub(ctx, &prep.main, &prep.fee)?;
                if <$name>::TYPE != TransactionType3::TYPE {
                    record_legacy_extra9_burn(ctx, &prep.fee, &self.fee_got())?;
                }
                crate::exec::tex::settlement_addr_postsettle_cleanup(ctx)?;
                Ok(())
            }
        }
    };
}

impl_tx_type_execute!(TransactionType1);
impl_tx_type_execute!(TransactionType2);
impl_tx_type_execute!(TransactionType3);
