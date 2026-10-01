//! Dependency graph: callers-of and callees-of, with edge kinds.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    /// A direct call.
    Call,
    /// The word's address escapes as a value (`'word`, quotation values).
    AddressTaken,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Edge {
    pub word: String,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Graph {
    pub callees_of: BTreeMap<String, BTreeSet<Edge>>,
    pub callers_of: BTreeMap<String, BTreeSet<Edge>>,
}

impl Graph {
    /// Replace all outgoing edges of `from`.
    pub fn set_callees(&mut self, from: &str, edges: impl IntoIterator<Item = Edge>) {
        if let Some(old) = self.callees_of.remove(from) {
            for e in old {
                if let Some(c) = self.callers_of.get_mut(&e.word) {
                    c.retain(|x| x.word != from);
                }
            }
        }
        let set: BTreeSet<Edge> = edges.into_iter().collect();
        for e in &set {
            self.callers_of
                .entry(e.word.clone())
                .or_default()
                .insert(Edge {
                    word: from.to_string(),
                    kind: e.kind,
                });
        }
        self.callees_of.insert(from.to_string(), set);
    }

    pub fn callers(&self, name: &str) -> Vec<String> {
        self.callers_of
            .get(name)
            .map(|s| {
                let names: BTreeSet<_> = s.iter().map(|e| e.word.clone()).collect();
                names.into_iter().collect()
            })
            .unwrap_or_default()
    }

    pub fn callees(&self, name: &str) -> Vec<Edge> {
        self.callees_of
            .get(name)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// All words reachable from `roots` along callee edges (including the roots).
    pub fn reachable<'a>(&self, roots: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut todo: Vec<String> = roots.into_iter().map(str::to_string).collect();
        while let Some(w) = todo.pop() {
            if seen.insert(w.clone()) {
                for e in self.callees(&w) {
                    todo.push(e.word);
                }
            }
        }
        seen
    }
}
