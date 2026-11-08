//! Un résultat par fichier. L'agent cherche des fichiers : les dix places montrent dix fichiers
//! différents, et les autres passages d'un fichier sont signalés sous son meilleur passage
//! (rejeu du banc : un même fichier occupait parfois les dix places).

use crate::types::{Hit, Passage};
use std::collections::HashMap;

/// Garde le premier (meilleur) passage de chaque fichier, dans l'ordre reçu ; les suivants
/// deviennent ses `also`, dans l'ordre du classement.
pub fn by_file(hits: Vec<Hit>) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    let mut slot: HashMap<String, usize> = HashMap::new();
    for hit in hits {
        match slot.get(&hit.path) {
            Some(&i) => out[i].also.push(Passage { start: hit.start, end: hit.end, name: hit.name }),
            None => {
                slot.insert(hit.path.clone(), out.len());
                out.push(hit);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str, start: usize, name: &str) -> Hit {
        Hit {
            id: start as u64,
            path: path.into(),
            start,
            end: start + 9,
            kind: "function".into(),
            name: name.into(),
            parent: String::new(),
            signature: String::new(),
            body: String::new(),
            score: 1.0,
            also: Vec::new(),
        }
    }

    #[test]
    fn un_resultat_par_fichier_dans_l_ordre_du_classement() {
        let hits = vec![hit("a.rs", 10, "f"), hit("b.rs", 1, "g"), hit("a.rs", 50, "h"), hit("c.rs", 5, "i"), hit("a.rs", 30, "j")];
        let out = by_file(hits);
        assert_eq!(out.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), ["a.rs", "b.rs", "c.rs"]);
        assert_eq!(out[0].start, 10, "le meilleur passage reste en tête");
        let also: Vec<(usize, &str)> = out[0].also.iter().map(|p| (p.start, p.name.as_str())).collect();
        assert_eq!(also, [(50, "h"), (30, "j")], "les autres passages suivent l'ordre du classement");
        assert!(out[1].also.is_empty() && out[2].also.is_empty());
    }
}
