//! Définition exacte d'un symbole : nom exact (ou `Parent::nom`), définitions avant les
//! fenêtres de lignes, code applicatif avant les tests.

use crate::bm25::Bm25;
use crate::fuse::is_test_path;
use crate::types::Hit;
use anyhow::Result;

pub fn find(index: &Bm25, query: &str, limit: usize) -> Result<Vec<Hit>> {
    let (parent, name) = match query.rsplit_once("::").or_else(|| query.rsplit_once('.')) {
        Some((p, n)) => (Some(p), n),
        None => (None, query),
    };
    let mut hits: Vec<Hit> = index
        .by_name(name.trim(), 200)?
        .into_iter()
        .filter(|h| h.kind != "lines")
        .filter(|h| parent.is_none_or(|p| h.parent.ends_with(p) || h.parent.contains(p)))
        .collect();
    hits.sort_by(|a, b| {
        (is_test_path(&a.path), rank(&a.kind), &a.path, a.start).cmp(&(is_test_path(&b.path), rank(&b.kind), &b.path, b.start))
    });
    hits.truncate(limit);
    Ok(hits)
}

/// Les types et fonctions avant les blocs `impl` et le reste.
fn rank(kind: &str) -> u8 {
    match kind {
        "struct" | "class" | "enum" | "trait" | "interface" | "type" | "union" => 0,
        "function" | "method" | "macro" | "const" => 1,
        _ => 2,
    }
}
