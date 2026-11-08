//! Symboles → morceaux indexés : en-tête (chemin, parent, signature, doc) + corps,
//! identifiant stable, redécoupe des corps trop longs (text-splitter).

use crate::parse;
use crate::types::{Chunk, Unit};
use std::collections::HashMap;
use text_splitter::{ChunkConfig, CodeSplitter, TextSplitter};

/// Taille maximale du corps d'un morceau, en caractères (~1 000 tokens).
pub const MAX_BODY_CHARS: usize = 4000;

/// Identifiant stable : ne change pas si des lignes bougent au-dessus du symbole.
pub fn stable_id(path: &str, qualified: &str, kind: &str, ordinal: usize) -> u64 {
    let key = format!("{path}\0{qualified}\0{kind}\0{ordinal}");
    let bytes = blake3::hash(key.as_bytes());
    u64::from_le_bytes(bytes.as_bytes()[..8].try_into().expect("8 octets"))
}

pub fn chunks(path: &str, lang: &str, units: Vec<Unit>) -> Vec<Chunk> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for unit in units {
        let parent = unit.parent.clone().unwrap_or_default();
        let qualified = match (&unit.parent, unit.kind.as_str()) {
            (_, "lines") => format!("@{}", unit.start),
            (Some(p), _) => format!("{p}::{}", unit.name),
            (None, _) => unit.name.clone(),
        };
        let key = format!("{qualified}\0{}", unit.kind);
        let ordinal = *seen.entry(key.clone()).and_modify(|n| *n += 1).or_insert(0);
        for (part, (start, end, body)) in pieces(lang, &unit).into_iter().enumerate() {
            let id = stable_id(path, &qualified, &unit.kind, ordinal * 1000 + part);
            let header = header(path, &parent, &unit);
            out.push(Chunk {
                id,
                path: path.to_string(),
                lang: lang.to_string(),
                kind: unit.kind.clone(),
                name: unit.name.clone(),
                parent: parent.clone(),
                start,
                end,
                signature: unit.signature.clone(),
                text: format!("{header}\n{body}"),
                body,
            });
        }
    }
    out
}

fn header(path: &str, parent: &str, unit: &Unit) -> String {
    let mut h = format!("{path}\n");
    if !parent.is_empty() {
        h.push_str(&format!("in {parent}\n"));
    }
    if unit.kind != "lines" {
        h.push_str(&format!("{} {}\n", unit.kind, unit.signature));
    }
    if !unit.doc.is_empty() {
        h.push_str(&unit.doc);
        h.push('\n');
    }
    h
}

/// (ligne début, ligne fin, corps) ; un seul morceau si le corps tient dans la limite.
fn pieces(lang: &str, unit: &Unit) -> Vec<(usize, usize, String)> {
    if unit.body.len() <= MAX_BODY_CHARS {
        return vec![(unit.start, unit.end, unit.body.clone())];
    }
    let config = || ChunkConfig::new(MAX_BODY_CHARS);
    let indices: Vec<(usize, String)> = match parse::grammar(lang).and_then(|g| CodeSplitter::new(g, config()).ok()) {
        Some(splitter) => splitter.chunk_indices(&unit.body).map(|(o, s)| (o, s.to_string())).collect(),
        None => TextSplitter::new(config()).chunk_indices(&unit.body).map(|(o, s)| (o, s.to_string())).collect(),
    };
    indices
        .into_iter()
        .map(|(offset, piece)| {
            let start = unit.start + unit.body[..offset].matches('\n').count();
            let end = start + piece.trim_end().matches('\n').count();
            (start, end.min(unit.end), piece)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(name: &str, start: usize, body: &str) -> Unit {
        Unit {
            kind: "function".into(),
            name: name.into(),
            parent: None,
            start,
            end: start + body.matches('\n').count(),
            signature: format!("fn {name}()"),
            doc: String::new(),
            body: body.into(),
        }
    }

    #[test]
    fn id_stable_quand_les_lignes_bougent() {
        let a = chunks("a.rs", "rust", vec![unit("f", 1, "fn f() {}")]);
        let b = chunks("a.rs", "rust", vec![unit("f", 30, "fn f() {}")]);
        assert_eq!(a[0].id, b[0].id);
        assert!(a[0].text.starts_with("a.rs\nfunction fn f()"));
    }

    #[test]
    fn corps_long_redecoupe_avec_lignes_coherentes() {
        let body = (0..400).map(|i| format!("    let v{i} = {i};")).collect::<Vec<_>>().join("\n");
        let body = format!("fn big() {{\n{body}\n}}");
        let c = chunks("a.rs", "rust", vec![unit("big", 10, &body)]);
        assert!(c.len() > 1);
        assert_eq!(c[0].start, 10);
        assert!(c.windows(2).all(|w| w[0].start <= w[1].start && w[0].id != w[1].id));
    }
}
