//! Banc d'essai : métriques au niveau fichier (acc@k, rappel@10, MRR, nDCG@10).
//! Jeu de données JSONL : `{"query": "...", "expected": ["chemin/relatif", ...]}`.

use crate::types::Hit;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Case {
    pub query: String,
    pub expected: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub cases: usize,
    pub acc_at_1: f64,
    pub acc_at_5: f64,
    pub acc_at_10: f64,
    pub recall_at_10: f64,
    pub mrr: f64,
    pub ndcg_at_10: f64,
    pub p50_ms: u64,
    pub p95_ms: u64,
}

pub fn load(path: &Path) -> Result<Vec<Case>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).with_context(|| format!("ligne {} invalide", i + 1)))
        .collect()
}

/// Fichiers distincts dans l'ordre du classement.
pub fn ranked_files(hits: &[Hit]) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for h in hits {
        if !files.contains(&h.path) {
            files.push(h.path.clone());
        }
    }
    files
}

pub fn run(cases: &[Case], mut search: impl FnMut(&str) -> Result<Vec<Hit>>) -> Result<Report> {
    let mut r = Report { cases: cases.len(), ..Default::default() };
    let mut latencies = Vec::new();
    for case in cases {
        let t = std::time::Instant::now();
        let files = ranked_files(&search(&case.query)?);
        latencies.push(t.elapsed().as_millis() as u64);
        let relevant = |f: &String| case.expected.iter().any(|e| e == f);
        let first = files.iter().position(relevant);
        let hit_at = |k: usize| f64::from(u8::from(first.is_some_and(|p| p < k)));
        r.acc_at_1 += hit_at(1);
        r.acc_at_5 += hit_at(5);
        r.acc_at_10 += hit_at(10);
        r.mrr += first.map_or(0.0, |p| 1.0 / (p as f64 + 1.0));
        let found = files.iter().take(10).filter(|f| relevant(f)).count();
        r.recall_at_10 += found as f64 / case.expected.len().max(1) as f64;
        let dcg: f64 = files.iter().take(10).enumerate().filter(|(_, f)| relevant(f)).map(|(i, _)| 1.0 / (i as f64 + 2.0).log2()).sum();
        let ideal: f64 = (0..case.expected.len().min(10)).map(|i| 1.0 / (i as f64 + 2.0).log2()).sum();
        r.ndcg_at_10 += if ideal > 0.0 { dcg / ideal } else { 0.0 };
    }
    let n = cases.len().max(1) as f64;
    for m in [&mut r.acc_at_1, &mut r.acc_at_5, &mut r.acc_at_10, &mut r.recall_at_10, &mut r.mrr, &mut r.ndcg_at_10] {
        *m /= n;
    }
    latencies.sort_unstable();
    let pct = |p: f64| latencies.get(((latencies.len() as f64 - 1.0) * p).round() as usize).copied().unwrap_or(0);
    r.p50_ms = pct(0.5);
    r.p95_ms = pct(0.95);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str) -> Hit {
        Hit {
            id: 0,
            path: path.into(),
            start: 1,
            end: 1,
            kind: String::new(),
            name: String::new(),
            parent: String::new(),
            signature: String::new(),
            body: String::new(),
            score: 0.0,
        }
    }

    #[test]
    fn metriques() {
        let cases = vec![
            Case { query: "a".into(), expected: vec!["x.rs".into()] },
            Case { query: "b".into(), expected: vec!["z.rs".into()] },
        ];
        let r = run(&cases, |q| Ok(if q == "a" { vec![hit("y.rs"), hit("x.rs")] } else { vec![hit("y.rs")] })).unwrap();
        assert_eq!((r.acc_at_1, r.acc_at_5, r.mrr), (0.0, 0.5, 0.25));
        assert!((r.ndcg_at_10 - 0.5 * (1.0 / 3f64.log2())).abs() < 1e-9);
    }
}
