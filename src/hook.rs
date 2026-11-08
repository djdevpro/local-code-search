//! Hook `UserPromptSubmit` de Claude Code : avant le premier tour de l'agent, cherche la demande de
//! l'utilisateur et lui glisse les fichiers probablement concernés (un aller-retour de moins).
//!
//! Entrée (stdin) : `{"prompt": "...", ...}`. Sortie (stdout) : `hookSpecificOutput.additionalContext`.
//! Ne bloque jamais la demande : toute erreur donne une sortie vide.

use crate::types::Hit;

/// Marqueur repris dans la section CLAUDE.md : l'agent sait d'où vient ce contexte.
pub const HEADER: &str = "Files probably involved (local-code-search, from the user's request)";

/// Le texte de la demande, s'il vaut une recherche : assez de mots, pas une commande `/…`.
pub fn prompt_to_search(stdin: &str, min_words: usize) -> Option<String> {
    let input: serde_json::Value = serde_json::from_str(stdin).ok()?;
    let prompt = input.get("prompt")?.as_str()?.trim();
    if prompt.starts_with('/') || prompt.split_whitespace().count() < min_words {
        return None;
    }
    // Les longues demandes (log collé, ticket détaillé) : le début porte l'essentiel.
    Some(prompt.chars().take(2000).collect())
}

/// Le JSON à écrire sur stdout, ou `None` quand il n'y a rien d'utile à dire.
pub fn output(hits: &[Hit]) -> Option<String> {
    if hits.is_empty() {
        return None;
    }
    let mut ctx = format!("{HEADER}:\n");
    for h in hits {
        let symbol = match (h.name.is_empty(), h.parent.is_empty()) {
            (true, _) => String::new(),
            (false, true) => format!("  {}", h.name),
            (false, false) => format!("  {}::{}", h.parent, h.name),
        };
        ctx.push_str(&format!("- {}:{}-{}{symbol}\n", h.path, h.start, h.end));
    }
    ctx.push_str(
        "Start from these files: open the relevant ones directly, and search further only if they do not cover \
         the request. They come from a search on the raw request, so they may miss a part of it.",
    );
    Some(
        serde_json::json!({
            "hookSpecificOutput": { "hookEventName": "UserPromptSubmit", "additionalContext": ctx }
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str, name: &str, parent: &str) -> Hit {
        Hit {
            id: 1,
            path: path.into(),
            start: 10,
            end: 20,
            kind: "function".into(),
            name: name.into(),
            parent: parent.into(),
            signature: String::new(),
            body: String::new(),
            score: 1.0,
            also: Vec::new(),
        }
    }

    #[test]
    fn ignore_les_demandes_courtes_les_commandes_et_le_json_invalide() {
        assert_eq!(prompt_to_search(r#"{"prompt": "ok vas-y"}"#, 6), None);
        assert_eq!(prompt_to_search(r#"{"prompt": "/review the whole branch please now"}"#, 6), None);
        assert_eq!(prompt_to_search("pas du json", 6), None);
        assert_eq!(prompt_to_search(r#"{"session_id": "x"}"#, 6), None);
        let q = prompt_to_search(r#"{"prompt": "la date de fin du projet est décalée d'un jour"}"#, 6);
        assert_eq!(q.as_deref(), Some("la date de fin du projet est décalée d'un jour"));
    }

    #[test]
    fn sortie_au_format_du_hook() {
        assert_eq!(output(&[]), None);
        let out = output(&[hit("src/a.rs", "f", "A"), hit("src/b.rs", "", "")]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.starts_with(HEADER) && ctx.contains("- src/a.rs:10-20  A::f\n- src/b.rs:10-20\n"), "{ctx}");
    }
}
