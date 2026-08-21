//! Recursive transaction action-tree analysis. `topology_facts` is the single
//! protocol-owned walk of scope / min-tx-type / flags / AST depth / top-rule; the execute path gates on the first finding, the SDK reports the full list.

use base::{ActScope, Action, ActionRef, ExecFrom, TopRule};
#[cfg(feature = "execute")]
use sys::{Rerr, errf};

fn is_guard_scope(scope: ActScope) -> bool {
    scope == ActScope::GUARD || scope == ActScope::TOP_GUARD_UNIQUE
}

#[derive(Default)]
struct Stats {
    findings: Vec<String>,
    action_notes: Vec<(usize, String)>,
    top_count: usize,
    top_kinds: std::collections::HashMap<u16, usize>,
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
        *stats.top_kinds.entry(act.kind()).or_insert(0) += 1;
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
        Some(TopRule::Unique) if stats.top_kinds.get(&act.kind()).copied().unwrap_or(0) != 1 => {
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
    use crate::codec::action::{EnvHeight, HacToTrs};
    use field::{Address, Amount, Uint2};
    use std::sync::Arc;

    fn transfer() -> ActionRef {
        Arc::new(HacToTrs::new(
            Address::from(*sys::Account::create_by("123456").unwrap().address()),
            Amount::from("1:244").unwrap(),
        ))
    }

    fn env_height() -> ActionRef {
        Arc::new(EnvHeight {
            kind: Uint2::from(EnvHeight::KIND),
        })
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
}
