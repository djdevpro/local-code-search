//! Fusion RRF pondérée et déterministe : même question, même ordre, toujours.

use crate::types::Hit;
use std::collections::HashMap;

pub fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let file = p.rsplit('/').next().unwrap_or(&p);
    p.split('/').any(|d| matches!(d, "test" | "tests" | "__tests__" | "spec" | "testdata"))
        || file.starts_with("test_")
        || file.contains("_test.")
        || file.contains(".test.")
        || file.contains(".spec.")
}

/// Applique la pénalité des fichiers de test puis retrie (score, chemin, ligne).
pub fn penalize_tests(mut hits: Vec<Hit>, factor: f32) -> Vec<Hit> {
    let is_test = |h: &Hit| is_test_path(&h.path) || h.parent == "tests" || h.parent.ends_with("::tests");
    for h in hits.iter_mut().filter(|h| is_test(h)) {
        // Fonctionne aussi pour des scores négatifs (reranker) : on retire toujours une part.
        h.score -= h.score.abs() * (1.0 - factor);
    }
    sort(&mut hits);
    hits
}

pub fn sort(hits: &mut [Hit]) {
    hits.sort_by(|a, b| {
        b.score.total_cmp(&a.score).then_with(|| a.path.cmp(&b.path)).then_with(|| a.start.cmp(&b.start))
    });
}

/// score = Σ poids / (k + rang) sur chaque liste où le morceau apparaît.
pub fn rrf(legs: Vec<(Vec<Hit>, f32)>, k: f32) -> Vec<Hit> {
    let mut fused: HashMap<u64, Hit> = HashMap::new();
    for (hits, weight) in legs {
        for (rank, hit) in hits.into_iter().enumerate() {
            let contribution = weight / (k + rank as f32 + 1.0);
            fused
                .entry(hit.id)
                .and_modify(|h| h.score += contribution)
                .or_insert(Hit { score: contribution, ..hit });
        }
    }
    let mut out: Vec<Hit> = fused.into_values().collect();
    sort(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: u64, path: &str) -> Hit {
        Hit {
            id,
            path: path.into(),
            start: 1,
            end: 2,
            kind: "function".into(),
            name: String::new(),
            parent: String::new(),
            signature: String::new(),
            body: String::new(),
            score: 1.0,
        }
    }

    #[test]
    fn present_dans_les_deux_listes_monte() {
        let bm25 = vec![hit(1, "a"), hit(2, "b"), hit(3, "c")];
        let vec = vec![hit(3, "c"), hit(4, "d")];
        let out = rrf(vec![(bm25, 1.0), (vec, 1.0)], 60.0);
        assert_eq!(out[0].id, 3);
    }

    #[test]
    fn ordre_deterministe_en_cas_egalite() {
        let out = rrf(vec![(vec![hit(9, "z")], 1.0), (vec![hit(8, "a")], 1.0)], 60.0);
        assert_eq!(out.iter().map(|h| h.id).collect::<Vec<_>>(), [8, 9]);
    }

    #[test]
    fn reconnait_les_tests() {
        assert!(is_test_path("src/tests/a.rs") && is_test_path("pkg/foo_test.go") && is_test_path("a.spec.ts"));
        assert!(!is_test_path("src/contest.rs"));
    }
}
