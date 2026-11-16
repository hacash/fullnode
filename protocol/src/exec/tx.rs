//! Transaction execute bodies for prelude + standard (types 1/2/3) txs.

use base::{
    ActionDispatcher, ActionRef, Context, CoreState, ExecFrom, Transaction, TransactionExecute,
    TransactionSign, hac_add, hac_sub,
};
use field::{Amount, Encode, Hash};
use sys::{Rerr, Ret, errf};

use crate::codec::tx::{DefaultPreludeTx, StdTransaction};

/// Tx-level caps for `TexCellRepeat` (kind 23): the per-action ceilings
/// (`check_repeat_action_caps`, from `params` — decode only enforces the
/// parameter-free floor `times ≥ 2`), then the tx-wide sums. The expanded-cell
/// cap sums repeat actions only — kind 22 cells are not counted, so historical
/// transactions (which can carry well over 1,024 kind-22 cells within the
/// 16KB limit) stay valid after activation. The expanded-action count charges
/// each repeat with its `times` and every other action with 1, which degrades
/// to the existing `actions.len() ≤ tx_actions_max` rule for repeat-free txs.
/// Mandatory here (execute runs under fast sync too); `precheck_tx` repeats it
/// only as an early failure.
fn check_tex_repeat_caps(params: &hacash_params::ProtocolParams, actions: &[ActionRef]) -> Rerr {
    let repeat_kind = crate::codec::action::tex::TexCellRepeat::KIND;
    let mut expanded_actions: usize = 0;
    let mut expanded_cells: usize = 0;
    for act in actions {
        if act.kind() != repeat_kind {
            expanded_actions += 1;
            continue;
        }
        let Some(rep) = act
            .as_any()
            .downcast_ref::<crate::codec::action::tex::TexCellRepeat>()
        else {
            return errf!("tx action kind {repeat_kind} is not TexCellRepeat");
        };
        crate::codec::action::tex::check_repeat_action_caps(params, rep)?;
        let times = rep.times() as usize;
        expanded_actions += times;
        expanded_cells += times.saturating_mul(rep.cells().len());
    }
    if expanded_actions > params.tx_actions_max {
        return errf!(
            "tx expanded actions exceed limit {} > {}",
            expanded_actions,
            params.tx_actions_max
        );
    }
    if expanded_cells > params.tex_repeat_cells_max_per_tx {
        return errf!(
            "tx repeat cells exceed limit {} > {}",
            expanded_cells,
            params.tex_repeat_cells_max_per_tx
        );
    }
    Ok(())
}

/// Kind 23 is refused on `[800_000, 802_000)` (`base::tex_repeat_enabled_at`).
/// Heights below the start stay open. No-op when the tx has no repeat, so
/// historical transactions are untouched. Mandatory in `execute_actions`
/// (fast sync skips `precheck_tx`).
fn check_tex_repeat_window(ctx: &dyn Context, actions: &[ActionRef]) -> Rerr {
    let repeat_kind = crate::codec::action::tex::TexCellRepeat::KIND;
    if !actions.iter().any(|act| act.kind() == repeat_kind) {
        return Ok(());
    }
    let height = ctx.env().block.height;
    if base::tex_repeat_enabled_at(height) {
        return Ok(());
    }
    errf!(
        "TexCellRepeat not enabled at height {height} (refused on [{}, {}))",
        base::TEX_REPEAT_DISABLE_FROM_HEIGHT,
        base::TEX_REPEAT_DISABLE_UNTIL_HEIGHT
    )
}

/// A `TexCellRepeat` (kind 23) always charges gas — `times ≥ 2` bills at least
/// one extra payload — so a type-3 envelope with `gas_max = 0` can never run
/// one: reject it in precheck instead of failing at execute time with
/// "gas not initialized" after the fee totals are already booked. Kind-22
/// non-asset txs keep their `gas_max = 0` path; only repeat-carrying txs are
/// constrained. Mandatory in `execute_actions` too (fast sync runs it there).
fn check_tex_repeat_gas(ctx: &dyn Context, actions: &[ActionRef]) -> Rerr {
    let repeat_kind = crate::codec::action::tex::TexCellRepeat::KIND;
    if !actions.iter().any(|act| act.kind() == repeat_kind) {
        return Ok(());
    }
    let params = crate::execution_params(ctx.services().as_ref())?;
    let budget = match ctx.tx().gas_max_byte() {
        Some(byte) => params.decode_gas_budget(byte.min(params.tx_gas_budget_cap_byte)),
        None => return errf!("tx type {} gas_max must exist", ctx.tx().ty()),
    };
    if budget <= 0 {
        return errf!(
            "transaction carrying TexCellRepeat (kind {}) requires gas_max > 0",
            repeat_kind
        );
    }
    Ok(())
}

fn precheck_tx(ctx: &dyn Context, tx: &dyn Transaction, actions: &[ActionRef]) -> Rerr {
    let params = crate::execution_params(ctx.services().as_ref())?;
    if tx.ty() == hacash_params::TX_TYPE_3 {
        if let Some(tx) = tx.as_any().downcast_ref::<StdTransaction>() {
            tx.validate_signer_limit(params.max_type3_signers)?;
        }
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
    check_tex_repeat_window(ctx, actions)?;
    check_tex_repeat_caps(params, actions)?;
    check_tex_repeat_gas(ctx, actions)?;
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
    let params = crate::execution_params(ctx.services().as_ref())?;
    check_tex_repeat_window(ctx, actions)?;
    check_tex_repeat_caps(params, actions)?;
    check_tex_repeat_gas(ctx, actions)?;
    let tex_kind = crate::codec::action::tex::TexCellExecute::KIND;
    let repeat_kind = crate::codec::action::tex::TexCellRepeat::KIND;
    let mut in_tex = false;
    let mut settled = false;
    for act in actions {
        let is_tex = act.kind() == tex_kind || act.kind() == repeat_kind;
        if in_tex && !is_tex {
            crate::exec::tex::do_settlement(ctx)?;
            in_tex = false;
            settled = true;
        }
        if is_tex && settled {
            return errf!("tex settlement can only run once per transaction");
        }
        ctx.exec_from_set(ExecFrom::Top);
        if is_tex {
            in_tex = true;
        }
        if charge_extra9 {
            let _ = ActionDispatcher::dispatch_top(ctx, act)?;
        } else {
            let _ = ActionDispatcher::dispatch_top_without_extra9(ctx, act)?;
        }
    }
    // The segment runs through the last action, so the loop never saw a
    // non-TEX boundary. A segment already closed, or a transaction with no
    // TEX, does not settle here.
    if in_tex {
        crate::exec::tex::do_settlement(ctx)?;
    }
    Ok(())
}

impl TransactionExecute for DefaultPreludeTx {
    fn execute(&self, ctx: &mut dyn Context) -> Rerr {
        hac_add(ctx, &self.address, &self.reward)?;
        Ok(())
    }
}

impl TransactionExecute for StdTransaction {
    fn execute(&self, ctx: &mut dyn Context) -> Rerr {
        precheck_tx(ctx, self, &self.actions)?;
        let prep = prepare_tx_execute(self, ctx)?;
        let is_type3 = self.ty.uint() == hacash_params::TX_TYPE_3;
        if !ctx.env().chain.fast_sync && !is_type3 && prep.has_ast_control {
            return errf!(
                "tx type {} cannot include AST control-flow actions; requires at least type 3",
                self.ty.uint()
            );
        }
        if !ctx.env().chain.fast_sync {
            if let Some(note) = crate::facts::ano_mark_finding(self.ty.uint(), self.ano_mark[0]) {
                return errf!("{}", note);
            }
            if let Some(note) = crate::facts::gas_max_finding_with_params(
                crate::execution_params(ctx.services().as_ref())?,
                self.ty.uint(),
                self.gas_max.uint(),
            ) {
                return errf!("{}", note);
            }
        }

        mark_tx_exist(ctx, &prep.tx_hash, prep.block_height);
        record_tx_fee_totals(ctx, self)?;
        if is_type3 {
            let gas_initialized = crate::exec::gas::tx_gas_initialize(ctx)?;
            execute_actions(ctx, &self.actions, true)?;
            ctx.run_deferred_phase()?;
            if gas_initialized {
                ctx.gas_refund()?;
            }
        } else {
            execute_actions(ctx, &self.actions, false)?;
        }
        hac_sub(ctx, &prep.main, &prep.fee)?;
        if !is_type3 {
            record_legacy_extra9_burn(ctx, &prep.fee, &self.fee_got())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::codec::action::tex::{TexCell, TexCellExecute, TexCellRepeat};
    use field::{Address, Fold64, ListW1, Sign, Uint2};

    fn repeat(times: u16, cells: usize) -> ActionRef {
        Arc::new(TexCellRepeat {
            kind: Uint2::from(TexCellRepeat::KIND),
            times: Uint2::from(times),
            addr: Address::default(),
            cells: ListW1::from(
                vec![
                    TexCell::ZhuPay {
                        haczhu: Fold64::from(1).unwrap(),
                    };
                    cells
                ],
            )
            .unwrap(),
            sign: Sign::default(),
        })
    }

    fn kind22() -> ActionRef {
        Arc::new(TexCellExecute {
            kind: Uint2::from(TexCellExecute::KIND),
            addr: Address::default(),
            cells: ListW1::from(vec![TexCell::ZhuPay {
                haczhu: Fold64::from(1).unwrap(),
            }])
            .unwrap(),
            sign: Sign::default(),
        })
    }

    fn caps(actions: &[ActionRef]) -> Rerr {
        check_tex_repeat_caps(&hacash_params::MAINNET_PARAMS.protocol, actions)
    }

    /// The tx-level caps: per-action ceilings from `params` (a Uint2-representable
    /// `times` decodes fine but is refused here), `Σ times` + one per other
    /// action vs `tx_actions_max`, and `Σ times × cells` over repeat actions
    /// ONLY vs the per-tx cell cap — kind-22 cells are never counted, so
    /// pre-repeat historical shapes (which can carry well over 1,024 kind-22
    /// cells within 16KB) stay valid.
    #[test]
    fn repeat_caps_sum_repeat_actions_only() {
        // Per-action times ceiling: 201 > tex_repeat_times_max 200.
        assert!(caps(&[repeat(201, 1)]).is_err());
        // 2×(100×5): Σtimes=200, Σcells=1,000 — both caps exactly met.
        caps(&[repeat(100, 5), repeat(100, 5)]).unwrap();
        // Σtimes=202 > 200.
        assert!(caps(&[repeat(101, 5), repeat(101, 5)]).is_err());
        // Σcells=1,200 > 1,024 while Σtimes=200 is fine: the cell cap binds.
        assert!(caps(&[repeat(100, 6), repeat(100, 6)]).is_err());
        // Historical safety: 200 plain kind-22 actions (300 kind-22 cells,
        // more per tx than any repeat tx may have) stay valid.
        let actions: Vec<ActionRef> = (0..200).map(|_| kind22()).collect();
        caps(&actions).unwrap();
        // Mixed counting: others count 1 each, the repeat counts its times.
        let mut actions: Vec<ActionRef> = (0..150).map(|_| kind22()).collect();
        actions.push(repeat(60, 1)); // 150 + 60 = 210 > 200
        assert!(caps(&actions).is_err());
        actions.pop();
        actions.push(repeat(50, 1)); // 150 + 50 = 200
        caps(&actions).unwrap();
    }
}
