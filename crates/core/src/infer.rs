//! Effect inference: which un-annotated words must still declare their
//! effect, and (later) the elaboration that infers the rest.

use std::collections::{HashMap, HashSet};

use crate::ast::{Body, Item, NodeKind};
use crate::check::{check_body, Ctx, Mode};
use crate::diag::{codes, Diagnostic, Location};
use crate::types::{Effect, Ty};

/// The most inputs inference will try before giving up.
const MAX_INPUTS: usize = 32;

/// Infer the effect of an un-annotated body: the smallest number of inputs
/// for which it checks (the rest of the stack is grown one slot at a time
/// while the body underflows), with any type the body leaves open
/// generalised to a type parameter `T`, `U`, ... The result is then checked
/// by the caller exactly as if it had been written.
pub fn infer_effect(
    ctx: &mut Ctx,
    name: &str,
    body: &Body,
    loc: &Location,
) -> Result<Effect, Diagnostic> {
    let mut last = None;
    for n in 0..=MAX_INPUTS {
        match check_body(ctx, name, Mode::Infer(n), body, loc, &[]) {
            Ok(effect) => return Ok(generalise(&effect)),
            Err(d) if d.code == codes::E_STACK_UNDERFLOW => last = Some(d),
            Err(d) => return Err(d),
        }
    }
    Err(last.expect("at least one attempt"))
}

/// Replace each remaining type variable by a parameter named in order of
/// first appearance: `T U V W X Y Z`, then `T1 T2 ...`.
fn generalise(e: &Effect) -> Effect {
    fn walk(t: &Ty, names: &mut HashMap<u32, String>) -> Ty {
        match t {
            Ty::Var(v) => {
                let n = names.len();
                let name = names.entry(*v).or_insert_with(|| param_name(n)).clone();
                Ty::Param(name)
            }
            Ty::Array(e) => Ty::Array(Box::new(walk(e, names))),
            Ty::Quot(q) => Ty::Quot(Box::new(Effect {
                inputs: q.inputs.iter().map(|t| walk(t, names)).collect(),
                outputs: q.outputs.iter().map(|t| walk(t, names)).collect(),
                row: q.row,
            })),
            _ => t.clone(),
        }
    }
    let mut names = HashMap::new();
    let inputs = e.inputs.iter().map(|t| walk(t, &mut names)).collect();
    let outputs = e.outputs.iter().map(|t| walk(t, &mut names)).collect();
    Effect::new(inputs, outputs)
}

fn param_name(n: usize) -> String {
    const FIRST: [&str; 7] = ["T", "U", "V", "W", "X", "Y", "Z"];
    match FIRST.get(n) {
        Some(s) => s.to_string(),
        None => format!("T{}", n - FIRST.len() + 1),
    }
}

/// Remove every un-annotated definition that must declare its effect,
/// reporting `E_NEEDS_EFFECT` for each: `export` words, `main`, and words
/// that call themselves or each other (cycles are found over all `groups`
/// together, so they may span files).
pub fn require_effects(groups: &mut [&mut Vec<Item>], diags: &mut Vec<Diagnostic>) {
    let defs: Vec<(&str, &Body)> = groups
        .iter()
        .flat_map(|g| g.iter())
        .filter_map(|it| match it {
            Item::Def { name, body, .. } => Some((name.as_str(), body)),
            _ => None,
        })
        .collect();
    let index: HashMap<&str, usize> = defs.iter().enumerate().map(|(i, (n, _))| (*n, i)).collect();
    let edges: Vec<Vec<usize>> = defs
        .iter()
        .map(|(_, body)| {
            let mut bound = HashSet::new();
            bound_names(body, &mut bound);
            let mut out = Vec::new();
            referenced(body, &bound, &index, &mut out);
            out.sort();
            out.dedup();
            out
        })
        .collect();
    let sccs = tarjan(&edges);
    let mut cycle: HashMap<usize, Vec<String>> = HashMap::new();
    for scc in &sccs {
        let recursive = scc.len() > 1 || edges[scc[0]].contains(&scc[0]);
        if recursive {
            let mut names: Vec<String> = scc.iter().map(|&i| defs[i].0.to_string()).collect();
            names.sort();
            for &i in scc {
                cycle.insert(i, names.clone());
            }
        }
    }
    let flagged: HashMap<String, Diagnostic> = groups
        .iter()
        .flat_map(|g| g.iter())
        .filter_map(|it| match it {
            Item::Def {
                name,
                effect: None,
                export,
                loc,
                ..
            } => {
                let msg = if *export {
                    format!("`{name}` is exported, so it must declare its effect: `export : {name} ( inputs -- outputs ) ... ;`")
                } else if name == "main" {
                    "`main` must declare its effect: `: main ( -- ) ... ;`".to_string()
                } else {
                    let names = cycle.get(&index[name.as_str()])?;
                    if names.len() == 1 {
                        format!("`{name}` calls itself, so its effect cannot be inferred; write it: `: {name} ( inputs -- outputs ) ... ;`")
                    } else {
                        let list: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
                        format!("{} call each other, so their effects cannot be inferred; declare them: `: {name} ( inputs -- outputs ) ... ;`", join_and(&list))
                    }
                };
                Some((
                    name.clone(),
                    Diagnostic::error(codes::E_NEEDS_EFFECT, msg, loc.clone()).with_word(name),
                ))
            }
            _ => None,
        })
        .collect();
    if flagged.is_empty() {
        return;
    }
    for g in groups.iter_mut() {
        g.retain(|it| match it {
            Item::Def {
                name, effect: None, ..
            } => match flagged.get(name) {
                Some(d) => {
                    diags.push(d.clone());
                    false
                }
                None => true,
            },
            _ => true,
        });
    }
}

fn join_and(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// Every name bound by `:>` anywhere in a body: locals shadow words.
fn bound_names(body: &Body, out: &mut HashSet<String>) {
    for node in body {
        match &node.kind {
            NodeKind::Bind { name, .. } => {
                out.insert(name.clone());
            }
            kind => {
                for b in children(kind) {
                    bound_names(b, out);
                }
            }
        }
    }
}

fn referenced(
    body: &Body,
    bound: &HashSet<String>,
    index: &HashMap<&str, usize>,
    out: &mut Vec<usize>,
) {
    for node in body {
        match &node.kind {
            NodeKind::Name(n) | NodeKind::Tick(n) => {
                let base = n.strip_suffix('!').unwrap_or(n);
                if !bound.contains(n) && !bound.contains(base) {
                    if let Some(&i) = index.get(n.as_str()) {
                        out.push(i);
                    }
                }
            }
            kind => {
                for b in children(kind) {
                    referenced(b, bound, index, out);
                }
            }
        }
    }
}

fn children(kind: &NodeKind) -> Vec<&Body> {
    match kind {
        NodeKind::Quote(b)
        | NodeKind::When(b)
        | NodeKind::Unless(b)
        | NodeKind::Times(b)
        | NodeKind::Each(b)
        | NodeKind::Map(b)
        | NodeKind::Filter(b)
        | NodeKind::Fold(b) => vec![b],
        NodeKind::If(a, b) | NodeKind::While(a, b) | NodeKind::Until(a, b) => vec![a, b],
        _ => Vec::new(),
    }
}

/// Strongly connected components (Tarjan).
fn tarjan(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct State<'a> {
        edges: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(s: &mut State, v: usize) {
        s.index[v] = Some(s.next);
        s.low[v] = s.next;
        s.next += 1;
        s.stack.push(v);
        s.on_stack[v] = true;
        for &w in &s.edges[v] {
            match s.index[w] {
                None => {
                    visit(s, w);
                    s.low[v] = s.low[v].min(s.low[w]);
                }
                Some(iw) if s.on_stack[w] => s.low[v] = s.low[v].min(iw),
                _ => {}
            }
        }
        if Some(s.low[v]) == s.index[v] {
            let mut scc = Vec::new();
            while let Some(w) = s.stack.pop() {
                s.on_stack[w] = false;
                scc.push(w);
                if w == v {
                    break;
                }
            }
            s.out.push(scc);
        }
    }
    let n = edges.len();
    let mut s = State {
        edges,
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for v in 0..n {
        if s.index[v].is_none() {
            visit(&mut s, v);
        }
    }
    s.out
}
