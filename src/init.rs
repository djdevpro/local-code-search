//! Branche l'outil dans Claude Code (`.mcp.json`, section CLAUDE.md) et Cursor
//! (`.cursor/mcp.json` si présent). Sauvegarde `.bak`, mode `--dry-run`, aucun secret,
//! aucune consigne d'envoi de données.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::Path;

const SERVER: &str = "local-code-search";
const BEGIN: &str = "<!-- local-code-search:début -->";
const END: &str = "<!-- local-code-search:fin -->";

const GUIDE: &str = "## Recherche de code (local-code-search)

- Pour **comprendre ou localiser** du code par le sens (« où est géré X ? », « comment marche Y ? ») :
  outil MCP `search`. Pour la définition d'un symbole précis : outil `def`.
- Pour une recherche **exhaustive ou littérale** (tous les appels de X, une chaîne exacte, une constante) :
  garder grep, il est plus fiable et moins cher.
- Les résultats donnent `fichier:ligne` : pour voir plus d'un passage, utiliser l'outil Read avec offset et
  limit (pas de commande shell), plutôt que de lire les fichiers entiers.";

/// Renvoie la liste des actions (effectuées, ou prévues en `--dry-run`).
pub fn init(root: &Path, exe: &Path, dry_run: bool) -> Result<Vec<String>> {
    let root = crate::walk::canonical(root)?;
    let server = json!({ "command": exe.display().to_string(), "args": ["serve", "--root", root.display().to_string()] });
    let mut actions = Vec::new();

    let mut targets = vec![root.join(".mcp.json")];
    if root.join(".cursor").is_dir() {
        targets.push(root.join(".cursor").join("mcp.json"));
    }
    for path in targets {
        let mut config: Value = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("JSON invalide : {}", path.display()))?,
            Err(_) => json!({}),
        };
        if config["mcpServers"][SERVER] == server {
            actions.push(format!("{} : déjà à jour", path.display()));
            continue;
        }
        if !config["mcpServers"].is_object() {
            config["mcpServers"] = json!({});
        }
        config["mcpServers"][SERVER] = server.clone();
        actions.push(format!("{} : serveur MCP « {SERVER} » enregistré", path.display()));
        if !dry_run {
            backup(&path)?;
            std::fs::write(&path, serde_json::to_string_pretty(&config)? + "\n")?;
        }
    }

    let claude_md = root.join("CLAUDE.md");
    let current = std::fs::read_to_string(&claude_md).unwrap_or_default();
    let section = format!("{BEGIN}\n{GUIDE}\n{END}");
    let updated = match (current.find(BEGIN), current.find(END)) {
        (Some(b), Some(e)) if e > b => format!("{}{section}{}", &current[..b], &current[e + END.len()..]),
        _ if current.is_empty() => format!("{section}\n"),
        _ => format!("{}\n\n{section}\n", current.trim_end()),
    };
    if updated == current {
        actions.push(format!("{} : déjà à jour", claude_md.display()));
    } else {
        actions.push(format!("{} : section « quand utiliser search ou grep » ajoutée", claude_md.display()));
        if !dry_run {
            backup(&claude_md)?;
            std::fs::write(&claude_md, updated)?;
        }
    }
    if dry_run {
        actions.push("--dry-run : aucun fichier modifié".into());
    }
    Ok(actions)
}

fn backup(path: &Path) -> Result<()> {
    if path.exists() {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        std::fs::copy(path, bak)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotent_et_conserve_le_contenu_existant() {
        let dir = std::env::temp_dir().join(format!("lcs-init-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "# Mon projet\n").unwrap();
        std::fs::write(dir.join(".mcp.json"), r#"{"mcpServers":{"autre":{"command":"x"}}}"#).unwrap();
        let exe = Path::new("lcs");
        init(&dir, exe, false).unwrap();
        let second = init(&dir, exe, false).unwrap();
        assert!(second.iter().filter(|a| a.contains("déjà à jour")).count() == 2, "{second:?}");
        let mcp: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(".mcp.json")).unwrap()).unwrap();
        assert!(mcp["mcpServers"]["autre"].is_object() && mcp["mcpServers"][SERVER].is_object());
        let md = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(md.starts_with("# Mon projet") && md.matches(BEGIN).count() == 1);
        assert!(dir.join("CLAUDE.md.bak").exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
