//! Recursive transaction action-tree analysis. `topology_facts` is the single
//! protocol-owned walk of scope / min-tx-type / flags / AST depth / top-rule; the execute path gates on the first finding, the SDK reports the full list.

use base::{ActScope, Action, ActionRef, ExecFrom, TopRule};
#[cfg(feature = "execute")]
use sys::{Rerr, errf};

fn is_guard_scope(scope: ActScope) -> bool {
    scope.is_guard()
}

#[derive(Default)]
struct Stats {
    findings: Vec<String>,
    action_notes: Vec<(usize, String)>,
    top_count: usize,
    /// Top-level kind → occurrence count. Kind spaces are small; a `Vec` with
    /// linear lookup keeps the hash-table machinery out of the wasm graph.
    top_kinds: Vec<(u16, usize)>,
    top_guards: usize,
    terminal_non_guards: usize,
    terminal_guards: bool,
}

/// Protocol topology findings for one transaction body. Empty `findings`
/// means the action tree is well-formed under the given flags/depth cap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopologyFacts {
    pub findings: Vec<String>,
    /// Top-level action index → the finding recorded while visiting that node
    /// (scope / min-tx-type / flags); the SDK marks per-action `protocol_valid`.
    pub action_notes: Vec<(usize, String)>,
}

fn visit(
    tx_type: u8,
    act: &dyn Action,
    from: ExecFrom,
    depth: usize,
    flags: Option<u64>,
    max_depth: usize,
    top_index: Option<usize>,
    stats: &mut Stats,
) {
    if !act.scope().allows(from) {
        let text = format!(
            "action node invalid: action {} with scope {} not allowed from {}",
            act.kind(),
            format!("{:?}", act.scope()),
            from
        );
        stats.findings.push(text.clone());
        if let Some(index) = top_index {
            stats.action_notes.push((index, text));
        }
        return;
    }
    if tx_type < act.min_tx_type() {
        let text = format!(
            "action node invalid: action {} requires tx type >= {} but current tx type is {}",
            act.kind(),
            act.min_tx_type(),
            tx_type
        );
        stats.findings.push(text.clone());
        if let Some(index) = top_index {
            stats.action_notes.push((index, text));
        }
        return;
    }
    if let Some(flags) = flags {
        if act.required_flags() & !flags != 0 {
            let text = format!("action kind {} not activated", act.kind());
            stats.findings.push(text.clone());
            if let Some(index) = top_index {
                stats.action_notes.push((index, text));
            }
            return;
        }
    }
    if from == ExecFrom::Top {
        stats.top_count += 1;
        match stats
            .top_kinds
            .iter_mut()
            .find(|(kind, _)| *kind == act.kind())
        {
            Some((_, count)) => *count += 1,
            None => stats.top_kinds.push((act.kind(), 1)),
        }
        if is_guard_scope(act.scope()) {
            stats.top_guards += 1;
        }
    }
    let Some(nested) = act.nested_actions() else {
        if is_guard_scope(act.scope()) {
            stats.terminal_guards = true;
        } else {
            stats.terminal_non_guards += 1;
        }
        return;
    };
    let next_depth = match depth.checked_add(nested.depth_inc) {
        Some(d) => d,
        None => {
            stats.findings.push("ast tree depth overflow".to_owned());
            return;
        }
    };
    if next_depth > max_depth {
        stats.findings.push(format!(
            "ast tree depth {} exceeded max {}",
            next_depth, max_depth
        ));
        return;
    }
    for sub in nested.flatten() {
        visit(
            tx_type,
            sub,
            ExecFrom::Ast,
            next_depth,
            flags,
            max_depth,
            None,
            stats,
        );
    }
}

fn check_top_rule(act: &dyn Action, stats: &mut Stats) {
    match act.scope().top_rule() {
        Some(TopRule::Only) if stats.top_count != 1 => {
            stats.findings.push(format!(
                "tx topology invalid: action {} must be the only top action",
                act.kind()
            ));
        }
        Some(TopRule::OnlyCanWithGuard)
            if stats.top_count != stats.top_guards + 1 || stats.terminal_non_guards != 1 =>
        {
            stats.findings.push(format!(
                "tx topology invalid: action {} can only combine with guard actions",
                act.kind()
            ));
        }
        Some(TopRule::Unique)
            if stats
                .top_kinds
                .iter()
                .find(|(kind, _)| *kind == act.kind())
                .map(|(_, count)| *count)
                .unwrap_or(0)
                != 1 =>
        {
            stats.findings.push(format!(
                "tx topology invalid: action {} must be unique in tx",
                act.kind()
            ));
        }
        _ => {}
    }
}

/// Analyse the action tree without gating. `flags = None` skips the
/// activation-flag check (the SDK inspect context has no consensus flags).
pub fn topology_facts(
    tx_type: u8,
    actions: &[ActionRef],
    flags: Option<u64>,
    max_depth: usize,
) -> TopologyFacts {
    topology_facts_with_action_limit(
        tx_type,
        actions,
        flags,
        max_depth,
        hacash_params::MAINNET_PARAMS.protocol.tx_actions_max,
    )
}

pub fn topology_facts_with_action_limit(
    tx_type: u8,
    actions: &[ActionRef],
    flags: Option<u64>,
    max_depth: usize,
    max_actions: usize,
) -> TopologyFacts {
    let mut stats = Stats::default();
    if actions.is_empty() || actions.len() > max_actions {
        stats
            .findings
            .push(format!("action length {} is invalid", actions.len()));
        return TopologyFacts {
            findings: stats.findings,
            action_notes: stats.action_notes,
        };
    }
    for (index, act) in actions.iter().enumerate() {
        visit(
            tx_type,
            act.as_ref(),
            ExecFrom::Top,
            0,
            flags,
            max_depth,
            Some(index),
            &mut stats,
        );
    }
    if stats.terminal_guards && stats.terminal_non_guards == 0 {
        stats
            .findings
            .push("tx topology invalid: tx actions cannot be all GUARD".to_owned());
    }
    for act in actions {
        check_top_rule(act.as_ref(), &mut stats);
    }
    TopologyFacts {
        findings: stats.findings,
        action_notes: stats.action_notes,
    }
}

#[cfg(feature = "execute")]
pub fn precheck_tx_actions(
    tx_type: u8,
    actions: &[ActionRef],
    flags: u64,
    max_depth: usize,
    max_actions: usize,
) -> Rerr {
    let facts =
        topology_facts_with_action_limit(tx_type, actions, Some(flags), max_depth, max_actions);
    if let Some(first) = facts.findings.first() {
        return errf!("{}", first);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::action::{EnvHeight, TransferHacTo};
    use base::ActionCodec;
    use field::{Address, Amount, Encode, Uint2};
    use std::any::Any;
    use std::sync::Arc;

    fn transfer() -> ActionRef {
        Arc::new(TransferHacTo::new(
            Address::from(*sys::Account::create_by("123456").unwrap().address()),
            Amount::from("1:244").unwrap(),
        ))
    }

    fn env_height() -> ActionRef {
        Arc::new(EnvHeight {
            kind: Uint2::from(EnvHeight::KIND),
        })
    }

    /// Stand-in for a chain action. This crate cannot see the action types `vm` and
    /// `mint-core` own, so the same kind / min-tx-type / scope combination is
    /// reproduced here; the concrete declarations are pinned by the drift tests in
    /// the crates that own them (`vm::action::contract`, `mint_core::action::asset`).
    #[derive(Debug)]
    struct StubAction {
        kind: u16,
        min_tx_type: u8,
        scope: ActScope,
    }

    impl Encode for StubAction {
        fn size(&self) -> usize {
            2
        }

        fn encode_to(&self, out: &mut Vec<u8>) {
            Uint2::from(self.kind).encode_to(out);
        }
    }

    impl ActionCodec for StubAction {
        fn kind(&self) -> u16 {
            self.kind
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl field::ToJSON for StubAction {
        fn to_json_fmt(&self, _fmt: &field::JSONFormater) -> String {
            format!("{{\"kind\":{}}}", self.kind)
        }
    }

    impl Action for StubAction {
        fn scope(&self) -> ActScope {
            self.scope
        }

        fn min_tx_type(&self) -> u8 {
            self.min_tx_type
        }
    }

    fn stub(kind: u16, min_tx_type: u8, scope: ActScope) -> ActionRef {
        Arc::new(StubAction {
            kind,
            min_tx_type,
            scope,
        })
    }

    fn asset_create() -> ActionRef {
        stub(16, 2, ActScope::TOP_UNIQUE)
    }

    fn contract_deploy() -> ActionRef {
        stub(40, 3, ActScope::TOP)
    }

    /// The deploy scope before the change, kept so the tests can show which
    /// combinations the change unlocks. No real build resolves to this anymore.
    fn contract_deploy_before() -> ActionRef {
        stub(40, 3, ActScope::TOP_ONLY_CAN_WITH_GUARD)
    }

    fn contract_deploy_unique() -> ActionRef {
        stub(40, 3, ActScope::TOP_UNIQUE)
    }

    fn contract_update() -> ActionRef {
        stub(41, 3, ActScope::TOP_ONLY_CAN_WITH_GUARD)
    }

    fn contract_main_call() -> ActionRef {
        stub(44, 3, ActScope::AST)
    }

    fn required_signers() -> ActionRef {
        stub(0x0414, 2, ActScope::TOP_GUARD_UNIQUE)
    }

    fn findings(actions: &[ActionRef]) -> Vec<String> {
        topology_facts(3, actions, None, 6).findings
    }

    #[test]
    fn call_only_host_opcode_at_top_is_a_topology_finding() {
        let facts = topology_facts(3, &[env_height()], None, 6);
        assert!(
            facts
                .findings
                .iter()
                .any(|f| f.contains("not allowed from")),
            "CALL_ONLY at top must be a finding, got {:?}",
            facts.findings
        );
    }

    #[test]
    fn ordinary_transfer_has_no_topology_findings() {
        let facts = topology_facts(2, &[transfer()], None, 6);
        assert!(facts.findings.is_empty(), "{:?}", facts.findings);
    }

    #[test]
    fn nested_actions_is_the_single_control_flow_walker() {
        // A host opcode and a transfer are leaves (no `nested_actions`); the AST
        // kinds implement it, so the topology walk never special-cases kind numbers.
        assert!(transfer().nested_actions().is_none());
        assert!(env_height().nested_actions().is_none());
    }

    #[test]
    fn asset_create_combines_with_contract_deploy() {
        // The combination the deploy change is for: register the asset and deploy the
        // contract that administers it in one tx. Both scopes were closed before
        // (asset create TOP_ONLY, deploy OnlyCanWithGuard), so this needs both changes.
        let actions = vec![asset_create(), contract_deploy()];
        assert!(
            findings(&actions).is_empty(),
            "asset create + deploy must combine, got {:?}",
            findings(&actions)
        );
        // ...while asset create alone still forbids a second asset create.
        let twice = vec![asset_create(), asset_create()];
        assert!(
            findings(&twice)
                .iter()
                .any(|f| f.contains("must be unique in tx")),
            "two AssetCreate actions must stay rejected, got {:?}",
            findings(&twice)
        );
    }

    #[test]
    fn guard_actions_combine_with_asset_create_and_with_deploy() {
        // RequiredSigners is the top-only guard companion: it rides along with one
        // non-guard top action and is not counted as a duplicate of it.
        for companion in [asset_create(), contract_deploy()] {
            let actions = vec![required_signers(), companion];
            assert!(
                findings(&actions).is_empty(),
                "guard + companion must combine, got {:?}",
                findings(&actions)
            );
        }
        // ...but guards alone are still rejected: no value-moving action.
        let guards_only = vec![required_signers()];
        assert!(
            findings(&guards_only)
                .iter()
                .any(|f| f.contains("cannot be all GUARD")),
            "guards alone must stay rejected, got {:?}",
            findings(&guards_only)
        );
    }

    #[test]
    fn deploy_transfer_main_call_is_the_pool_funding_combination() {
        // Create the pool, fund it, then call into it. Which is newly legal because
        // transfer is CALL scope and main call is AST: both already allowed top, so
        // the deploy rule was the only blocker.
        let actions = vec![contract_deploy(), transfer(), contract_main_call()];
        assert!(
            findings(&actions).is_empty(),
            "deploy + transfer + main call must combine, got {:?}",
            findings(&actions)
        );
        // CONTRAST: the previous deploy scope rejected exactly this tx.
        let before = vec![contract_deploy_before(), transfer(), contract_main_call()];
        assert!(
            findings(&before)
                .iter()
                .any(|f| f.contains("can only combine with guard actions")),
            "the old deploy scope must be shown to reject the combination, got {:?}",
            findings(&before)
        );
    }

    #[test]
    fn contract_deploy_repeats_for_factory_batches() {
        // TOP rather than TOP_UNIQUE: a factory batches `[Deploy, Deploy, ...]` in one
        // tx, and the per-tx action cap plus each deploy's protocol cost bound it.
        let batch = vec![contract_deploy(), contract_deploy(), contract_deploy()];
        assert!(
            findings(&batch).is_empty(),
            "a deploy batch must be allowed, got {:?}",
            findings(&batch)
        );
        // CONTRAST: under TOP_UNIQUE every batch is rejected — which is why that rule
        // was not the right fit for deploy.
        let unique = vec![contract_deploy_unique(), contract_deploy_unique()];
        assert!(
            findings(&unique)
                .iter()
                .any(|f| f.contains("must be unique in tx")),
            "TOP_UNIQUE must be shown to reject the batch, got {:?}",
            findings(&unique)
        );
    }

    #[test]
    fn contract_update_still_combines_only_with_guards() {
        // The deploy change must not open the update path: ContractUpdate keeps its
        // own OnlyCanWithGuard rule, so deploy + update stays a finding.
        let blocked = vec![contract_deploy(), contract_update()];
        assert!(
            findings(&blocked)
                .iter()
                .any(|f| f.contains("can only combine with guard actions")),
            "deploy + update must stay rejected, got {:?}",
            findings(&blocked)
        );
        // update with a guard companion remains the one allowed shape
        let allowed = vec![required_signers(), contract_update()];
        assert!(findings(&allowed).is_empty(), "{:?}", findings(&allowed));
    }
}
