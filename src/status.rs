//! État de l'index et compteurs mesurés (jamais de « tokens économisés » estimés).

use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE: &str = "stats.json";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Stats {
    pub searches: u64,
    pub searches_with_hits: u64,
    pub total_search_ms: u64,
    pub last_index_ms: u64,
    pub last_index_files: u64,
}

pub fn load(data: &Path) -> Stats {
    std::fs::read_to_string(data.join(FILE)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(data: &Path, stats: &Stats) {
    if let Ok(json) = serde_json::to_string_pretty(stats) {
        let _ = std::fs::write(data.join(FILE), json);
    }
}

pub fn record_search(data: &Path, ms: u64, had_hits: bool) {
    let mut s = load(data);
    s.searches += 1;
    s.searches_with_hits += u64::from(had_hits);
    s.total_search_ms += ms;
    save(data, &s);
}

pub struct Info<'a> {
    pub root: &'a Path,
    pub files: u64,
    pub chunks: u64,
    pub vectors: Option<usize>,
    pub model: &'a str,
    pub backend: Option<&'a str>,
    pub disk_bytes: u64,
    pub stats: Stats,
}

pub fn render(i: &Info) -> String {
    let coverage = match i.vectors {
        Some(v) if i.chunks > 0 => format!("{v} ({:.0} % des morceaux)", v as f64 * 100.0 / i.chunks as f64),
        Some(v) => v.to_string(),
        None => "aucun (modèle jamais chargé)".into(),
    };
    let s = &i.stats;
    let avg = s.total_search_ms.checked_div(s.searches).unwrap_or(0);
    format!(
        "Projet      : {}\nFichiers    : {}\nMorceaux    : {}\nVecteurs    : {coverage}\nModèle      : {}\nBackend     : {}\nIndex       : {:.1} Mo\nRecherches  : {} ({} avec résultats, {avg} ms en moyenne)\nDernière indexation : {} fichiers en {} ms\n",
        i.root.display(),
        i.files,
        i.chunks,
        i.model,
        i.backend.unwrap_or("non chargé"),
        i.disk_bytes as f64 / 1e6,
        s.searches,
        s.searches_with_hits,
        s.last_index_files,
        s.last_index_ms,
    )
}

pub fn dir_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}
