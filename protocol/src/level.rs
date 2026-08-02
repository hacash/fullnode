//! Recursive transaction action-tree validation.

use base::{ActScope, Action, ActionRef, ExecFrom, TX_ACTIONS_MAX, TopRule};
use sys::{Rerr, errf};

fn is_guard_scope(scope: ActScope) -> bool {
    scope == ActScope::GUARD || scope == ActScope::TOP_GUARD_UNIQUE
}

#[derive(Default)]
struct Stats {
    top_count: usize,
    top_kinds: std::collections::HashMap<u16, usize>,
    top_guards: usize,
    terminal_non_guards: usize,
    terminal_guards: bool,
}

fn children(act: &dyn Action) -> Option<(usize, Vec<&dyn Action>)> {
    if let Some(ast) = act
        .as_any()
        .downcast_ref::<crate::codec::action::AstSelect>()
    {
        return Some((1, ast.child_actions()));
    }
    if let Some(ast) = act.as_any().downcast_ref::<crate::codec::action::AstIf>() {
        return Some((2, ast.child_actions()));
    }
    None
}

fn visit(
    tx_type: u8,
    act: &dyn Action,
    from: ExecFrom,
    depth: usize,
    flags: u64,
    max_depth: usize,
    stats: &mut Stats,
) -> Rerr {
    if !act.scope().allows(from) {
        return errf!(
            "action node invalid: action {} with scope {} not allowed from {}",
            act.kind(),
            format!("{:?}", act.scope()),
            from
        );
    }
    if tx_type < act.min_tx_type() {
        return errf!(
            "action node invalid: action {} requires tx type >= {} but current tx type is {}",
            act.kind(),
            act.min_tx_type(),
            tx_type
        );
    }
    if act.required_flags() & !flags != 0 {
        return errf!("action kind {} not activated", act.kind());
    }
    if from == ExecFrom::Top {
        stats.top_count += 1;
        *stats.top_kinds.entry(act.kind()).or_insert(0) += 1;
        if is_guard_scope(act.scope()) {
            stats.top_guards += 1;
        }
    }
    let Some((depth_inc, subs)) = children(act) else {
        if is_guard_scope(act.scope()) {
            stats.terminal_guards = true;
        } else {
            stats.terminal_non_guards += 1;
        }
        return Ok(());
    };
    let next_depth = depth
        .checked_add(depth_inc)
        .ok_or_else(|| "ast tree depth overflow".to_owned())?;
    if next_depth > max_depth {
        return errf!("ast tree depth {} exceeded max {}", next_depth, max_depth);
    }
    for sub in subs {
        visit(
            tx_type,
            sub,
            ExecFrom::Ast,
            next_depth,
            flags,
            max_depth,
            stats,
        )?;
    }
    Ok(())
}

fn check_top_rule(act: &dyn Action, stats: &Stats) -> Rerr {
    match act.scope().top_rule() {
        Some(TopRule::Only) if stats.top_count != 1 => {
            errf!(
                "tx topology invalid: action {} must be the only top action",
                act.kind()
            )
        }
        Some(TopRule::OnlyCanWithGuard)
            if stats.top_count != stats.top_guards + 1 || stats.terminal_non_guards != 1 =>
        {
            errf!(
                "tx topology invalid: action {} can only combine with guard actions",
                act.kind()
            )
        }
        Some(TopRule::Unique) if stats.top_kinds.get(&act.kind()).copied().unwrap_or(0) != 1 => {
            errf!(
                "tx topology invalid: action {} must be unique in tx",
                act.kind()
            )
        }
        _ => Ok(()),
    }
}

pub fn precheck_tx_actions(
    tx_type: u8,
    actions: &[ActionRef],
    flags: u64,
    max_depth: usize,
) -> Rerr {
    if actions.is_empty() || actions.len() > TX_ACTIONS_MAX {
        return errf!("action length {} is invalid", actions.len());
    }
    let mut stats = Stats::default();
    for act in actions {
        visit(
            tx_type,
            act.as_ref(),
            ExecFrom::Top,
            0,
            flags,
            max_depth,
            &mut stats,
        )?;
    }
    if stats.terminal_guards && stats.terminal_non_guards == 0 {
        return errf!("tx topology invalid: tx actions cannot be all GUARD");
    }
    for act in actions {
        check_top_rule(act.as_ref(), &stats)?;
    }
    Ok(())
}
