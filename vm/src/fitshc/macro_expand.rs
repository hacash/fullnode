use super::state::MacroDef;
use crate::rt::Token;
use crate::rt::Token::*;
use std::collections::HashSet;
use sys::{errf, Ret};

const MAX_EXPANSIONS: usize = 256;

pub(super) fn expand_macros(
    tokens: Vec<Token>,
    macros: &[MacroDef],
    reserved_names: &[String],
) -> Ret<Vec<Token>> {
    let mut tokens = tokens;
    let mut occupied = HashSet::new();
    occupied.extend(reserved_names.iter().cloned());
    for token in &tokens {
        if let Identifier(name) = token {
            occupied.insert(name.clone());
        }
    }
    for def in macros {
        for token in &def.body {
            if let Identifier(name) = token {
                occupied.insert(name.clone());
            }
        }
    }
    let mut expansion_id = 0usize;
    let mut expansions = 0usize;
    let mut idx = 0usize;
    while idx < tokens.len() {
        if let Some((name, args, after)) = invocation(&tokens, idx)? {
            let def = macros
                .iter()
                .find(|m| m.name == name)
                .ok_or_else(|| format!("unknown macro '{}'", name))?;
            if args.len() != def.params.len() {
                return errf!(
                    "macro '{}' expects {} arguments, got {}",
                    name,
                    def.params.len(),
                    args.len()
                );
            }
            if args
                .iter()
                .any(|arg| !matches!(arg.as_slice(), [Identifier(_)]))
            {
                return errf!("macro '{}' arguments must be variable names", name);
            }
            expansions += 1;
            if expansions > MAX_EXPANSIONS {
                return errf!("macro expansion limit exceeded (possible recursion)");
            }
            let mut substitutions = Vec::with_capacity(args.len());
            for (param, arg) in def.params.iter().zip(args) {
                substitutions.push((param.as_str(), arg[0].clone()));
            }
            let mut replacement = def.body.clone();
            hygienize_locals(&mut replacement, expansion_id, &mut occupied);
            for token in &mut replacement {
                if let Identifier(name) = token {
                    if let Some((_, value)) = substitutions.iter().find(|(param, _)| param == name)
                    {
                        *token = value.clone();
                    }
                }
            }
            expansion_id += 1;
            tokens.splice(idx..after, replacement);
        } else {
            idx += 1;
        }
    }
    Ok(tokens)
}

fn invocation(tokens: &[Token], idx: usize) -> Ret<Option<(String, Vec<Vec<Token>>, usize)>> {
    if !matches!(tokens.get(idx), Some(Identifier(word)) if word == "macro") {
        return Ok(None);
    }
    let Some(Identifier(name)) = tokens.get(idx + 1) else {
        return Ok(None);
    };
    if !matches!(tokens.get(idx + 2), Some(Partition('('))) {
        return Ok(None);
    }
    let mut args = Vec::new();
    let mut current = Vec::new();
    let mut stack = Vec::new();
    for (offset, token) in tokens.iter().enumerate().skip(idx + 3) {
        match token {
            Partition('(' | '[' | '{') => {
                stack.push(token.clone());
                current.push(token.clone());
            }
            Partition(')' | ']' | '}') => {
                if stack.is_empty() && matches!(token, Partition(')')) {
                    if !current.is_empty() || !args.is_empty() {
                        args.push(current);
                    }
                    return Ok(Some((name.clone(), args, offset + 1)));
                }
                match (stack.pop(), token) {
                    (Some(Partition('(')), Partition(')'))
                    | (Some(Partition('[')), Partition(']'))
                    | (Some(Partition('{')), Partition('}')) => current.push(token.clone()),
                    _ => return errf!("mismatched delimiter in macro invocation"),
                }
            }
            Partition(',') if stack.is_empty() => {
                args.push(std::mem::take(&mut current));
            }
            _ => current.push(token.clone()),
        }
    }
    errf!("unterminated macro invocation '{}'", name)
}

fn hygienize_locals(tokens: &mut [Token], expansion_id: usize, occupied: &mut HashSet<String>) {
    let mut private = Vec::<String>::new();
    for idx in 0..tokens.len().saturating_sub(1) {
        if matches!(
            tokens[idx],
            Keyword(
                crate::rt::KwTy::Var
                    | crate::rt::KwTy::Let
                    | crate::rt::KwTy::Const
                    | crate::rt::KwTy::Bind
            )
        ) {
            if let Identifier(name) = &tokens[idx + 1] {
                if !name.starts_with('$') {
                    private.push(name.clone());
                }
            }
        }
    }
    let mut renames = Vec::<(String, String)>::new();
    for name in private {
        if renames.iter().any(|(original, _)| original == &name) {
            continue;
        }
        let mut attempt = 0usize;
        let generated = loop {
            let suffix = if attempt == 0 {
                format!("__macro_{}", expansion_id)
            } else {
                format!("__macro_{}_{}", expansion_id, attempt)
            };
            let candidate = format!("{}{}", name, suffix);
            if occupied.insert(candidate.clone()) {
                break candidate;
            }
            attempt += 1;
        };
        renames.push((name, generated));
    }
    let mut previous_is_dot = false;
    for token in tokens.iter_mut() {
        if previous_is_dot {
            previous_is_dot = matches!(token, Keyword(crate::rt::KwTy::Dot));
            continue;
        }
        if let Identifier(name) = token {
            if let Some((_, generated)) = renames.iter().find(|(original, _)| original == name) {
                *name = generated.clone();
            }
        }
        previous_is_dot = matches!(token, Keyword(crate::rt::KwTy::Dot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_local_renaming_preserves_member_names() {
        let mut tokens = vec![
            Keyword(crate::rt::KwTy::Var),
            Identifier("c".into()),
            Keyword(crate::rt::KwTy::Assign),
            Integer(1),
            Identifier("Core".into()),
            Keyword(crate::rt::KwTy::Dot),
            Identifier("c".into()),
            Partition('('),
            Partition(')'),
        ];
        let mut occupied = HashSet::from(["c".to_owned(), "Core".to_owned()]);

        hygienize_locals(&mut tokens, 4, &mut occupied);

        assert_eq!(tokens[1], Identifier("c__macro_4".into()));
        assert_eq!(tokens[6], Identifier("c".into()));
    }
}
