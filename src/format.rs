//! Réponse compacte pour l'agent : définitions d'abord, extrait ciblé sur la partie qui
//! correspond, budget de tokens, repli en simples `fichier:ligne` au-delà.

use crate::split;
use crate::types::Hit;
use std::collections::HashSet;

const SNIPPET_LINES: usize = 12;
/// Autres passages du même fichier annoncés sous un résultat (au-delà : « +N »).
const ALSO_SHOWN: usize = 3;

/// `notes` : dégradations à annoncer (jamais silencieuses).
pub fn render(query: &str, hits: &[Hit], notes: &[String], budget: usize, count: &dyn Fn(&str) -> usize) -> String {
    let mut out = String::new();
    for n in notes {
        out.push_str(&format!("⚠ {n}\n"));
    }
    if hits.is_empty() {
        out.push_str(&format!("Aucun résultat pour « {query} ».\n"));
        return out;
    }
    let terms = query_terms(query);
    let mut used = count(&out);
    let mut overflow = Vec::new();
    for (i, hit) in hits.iter().enumerate() {
        let block = block(i + 1, hit, &terms);
        let cost = count(&block);
        if overflow.is_empty() && (used + cost <= budget || i == 0) {
            out.push_str(&block);
            used += cost;
        } else {
            overflow.push(locator(hit));
        }
    }
    if !overflow.is_empty() {
        out.push_str("Autres résultats (budget atteint) :\n");
        for l in overflow {
            out.push_str(&format!("- {l}\n"));
        }
    }
    out
}

pub fn locator(hit: &Hit) -> String {
    format!("{}:{}-{}  {}", hit.path, hit.start, hit.end, hit.signature)
}

fn block(rank: usize, hit: &Hit, terms: &HashSet<String>) -> String {
    let mut label = hit.kind.clone();
    if !hit.name.is_empty() {
        label.push_str(&format!(" {}", hit.name));
    }
    if !hit.parent.is_empty() {
        label.push_str(&format!(", dans {}", hit.parent));
    }
    let mut s = format!("{rank}. {}:{}-{}  [{label}]\n", hit.path, hit.start, hit.end);
    s.push_str(&snippet(hit, terms));
    if !hit.also.is_empty() {
        // Le nom seulement s'il change (un long symbole découpé répète le sien), une ligne seule sans tiret.
        let shown: Vec<String> = hit
            .also
            .iter()
            .take(ALSO_SHOWN)
            .map(|p| {
                let lines = if p.start == p.end { p.start.to_string() } else { format!("{}-{}", p.start, p.end) };
                match p.name.is_empty() || p.name == hit.name {
                    true => lines,
                    false => format!("{lines} {}", p.name),
                }
            })
            .collect();
        let more = hit.also.len().saturating_sub(ALSO_SHOWN);
        let more = if more > 0 { format!(" (+{more})") } else { String::new() };
        s.push_str(&format!("   aussi dans ce fichier : {}{more}\n", shown.join(", ")));
    }
    s.push('\n');
    s
}

/// La fenêtre de lignes du symbole qui contient le plus de termes de la question.
fn snippet(hit: &Hit, terms: &HashSet<String>) -> String {
    let lines: Vec<&str> = hit.body.lines().collect();
    let score = |l: &str| split::tokens(l).iter().filter(|(w, subs)| terms.contains(w) || subs.iter().any(|s| terms.contains(s))).count();
    let scores: Vec<usize> = lines.iter().map(|l| score(l)).collect();
    let n = SNIPPET_LINES.min(lines.len());
    let best = (0..=lines.len() - n).max_by_key(|&i| (scores[i..i + n].iter().sum::<usize>(), std::cmp::Reverse(i))).unwrap_or(0);
    let numbered = lines.len() == hit.end + 1 - hit.start;
    let mut s = String::new();
    if best > 0 {
        s.push_str("   …\n");
    }
    for (offset, line) in lines[best..best + n].iter().enumerate() {
        match numbered {
            true => s.push_str(&format!("   {:>5} │ {line}\n", hit.start + best + offset)),
            false => s.push_str(&format!("   {line}\n")),
        }
    }
    if best + n < lines.len() {
        s.push_str("   …\n");
    }
    s
}

fn query_terms(query: &str) -> HashSet<String> {
    split::tokens(query).into_iter().flat_map(|(w, subs)| std::iter::once(w).chain(subs)).collect()
}

/// Estimation de secours quand le tokenizer du modèle n'est pas disponible.
pub fn approx_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(body: &str, start: usize) -> Hit {
        Hit {
            id: 1,
            path: "src/a.rs".into(),
            start,
            end: start + body.lines().count() - 1,
            kind: "function".into(),
            name: "f".into(),
            parent: String::new(),
            signature: "fn f()".into(),
            body: body.into(),
            score: 1.0,
            also: Vec::new(),
        }
    }

    #[test]
    fn annonce_les_autres_passages_du_fichier() {
        let mut h = hit("fn f() {}", 1);
        h.also = (0..5).map(|i| crate::types::Passage { start: 10 * (i + 2), end: 10 * (i + 2) + 5, name: format!("g{i}") }).collect();
        h.also[1].name.clear();
        h.also[2] = crate::types::Passage { start: 40, end: 40, name: "f".into() };
        let out = render("f", &[h], &[], 10_000, &approx_tokens);
        assert!(out.contains("   aussi dans ce fichier : 20-25 g0, 30-35, 40 (+2)\n"), "{out}");
    }

    #[test]
    fn extrait_centre_sur_la_partie_qui_correspond() {
        let body = (1..=40).map(|i| if i == 30 { "let pool = connect();".to_string() } else { format!("x{i}();") }).collect::<Vec<_>>().join("\n");
        let out = render("connection pool", &[hit(&body, 100)], &[], 10_000, &approx_tokens);
        assert!(out.contains("129 │ let pool = connect();"), "{out}");
        assert!(out.starts_with("1. src/a.rs:100-139"));
    }

    #[test]
    fn budget_replie_en_localisateurs() {
        let hits: Vec<Hit> = (0..5).map(|i| hit(&"code();\n".repeat(20), i * 50 + 1)).collect();
        let out = render("code", &hits, &["vecteurs incomplets".into()], 60, &approx_tokens);
        assert!(out.starts_with("⚠ vecteurs incomplets"));
        assert!(out.contains("Autres résultats (budget atteint)"));
    }
}
