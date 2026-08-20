//! Recursive transaction action-tree analysis.
//!
//! `topology_facts` is the single protocol-owned walk of scope / min-tx-type /
//! flags / AST depth / top-rule. The execute path (`precheck_tx_actions`)
//! gates on the first finding; the SDK reports the full finding list as
//! review facts and never refuses to inspect or build because of them.

use base::{ActScope, Action, ActionRef, ExecFrom, TX_ACTIONS_MAX, TopRule};
use sys::{Rerr, errf};

fn is_guard_scope(scope: ActScope) -> bool {
    scope == ActScope::GUARD || scope == ActScope::TOP_GUARD_UNIQUE
}

#[derive(Default)]
struct Stats {
    findings: Vec<String>,
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
}

fn visit(
    tx_type: u8,
    act: &dyn Action,
    from: ExecFrom,
    depth: usize,
    flags: Option<u64>,
    max_depth: usize,
    stats: &mut Stats,
) {
    if !act.scope().allows(from) {
        stats.findings.push(format!(
            "action node invalid: action {} with scope {} not allowed from {}",
            act.kind(),
            format!("{:?}", act.scope()),
            from
        ));
        return;
    }
    if tx_type < act.min_tx_type() {
        stats.findings.push(format!(
            "action node invalid: action {} requires tx type >= {} but current tx type is {}",
            act.kind(),
            act.min_tx_type(),
            tx_type
        ));
        return;
    }
    if let Some(flags) = flags {
        if act.required_flags() & !flags != 0 {
            stats
                .findings
                .push(format!("action kind {} not activated", act.kind()));
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
    let mut stats = Stats::default();
    if actions.is_empty() || actions.len() > TX_ACTIONS_MAX {
        stats
            .findings
            .push(format!("action length {} is invalid", actions.len()));
        return TopologyFacts {
            findings: stats.findings,
        };
    }
    for act in actions {
        visit(
            tx_type,
            act.as_ref(),
            ExecFrom::Top,
            0,
            flags,
            max_depth,
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
    }
}

pub fn precheck_tx_actions(
    tx_type: u8,
    actions: &[ActionRef],
    flags: u64,
    max_depth: usize,
) -> Rerr {
    let facts = topology_facts(tx_type, actions, Some(flags), max_depth);
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
        // A host opcode is a leaf: no nested_actions. A transfer is a leaf.
        // The AST kinds implement the trait method; this pins that the
        // topology walk never special-cases kind numbers.
        assert!(transfer().nested_actions().is_none());
        assert!(env_height().nested_actions().is_none());
    }
}
