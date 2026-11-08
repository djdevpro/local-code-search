//! Branche lcs dans les agents de code : Claude Code (toujours), Codex et Cursor (s'ils sont présents).
//! Les réglages existants sont gardés, chaque fichier modifié a sa sauvegarde `.bak`.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SERVER: &str = "local-code-search";
const BEGIN: &str = "<!-- local-code-search:début -->";
const END: &str = "<!-- local-code-search:fin -->";

/// Consignes pour l'agent (CLAUDE.md, AGENTS.md, règle Cursor).
const GUIDE: &str = "## Code search (local-code-search)

- To **locate or understand** code by meaning (\"where is X handled?\", \"how does Y work?\"): MCP tool
  `search`. For the definition of a given symbol: tool `def`.
- Write `search` queries **in the language of the code** (usually English) and with its words (class and
  function names, technical terms), even when the request is in another language: translate first.
- Trust the results: open only the files you will change or must understand, and only the lines you need
  (in Claude Code, the Read tool with offset and limit rather than a shell command). Do not grep again to
  confirm a file `search` already found. For several aspects of a request, run several searches in the
  same turn.
- For an **exhaustive or literal** search (every call of X, an exact string, a constant): keep grep, it is
  more reliable and cheaper.";

/// Agents de code à brancher en plus de Claude Code.
pub struct Harnesses {
    pub codex: bool,
    pub cursor: bool,
}

impl Harnesses {
    /// Présents dans le projet (`.codex/`, `.cursor/`) ou installés (`~/.codex`, `~/.cursor`, `CODEX_HOME`).
    fn detect(root: &Path) -> Self {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
        let present = |name: &str| root.join(name).is_dir() || home.as_ref().is_some_and(|h| h.join(name).is_dir());
        let codex_home = std::env::var_os("CODEX_HOME").is_some_and(|p| Path::new(&p).is_dir());
        Harnesses { codex: present(".codex") || codex_home, cursor: present(".cursor") }
    }
}

/// Renvoie les actions effectuées (ou prévues en `--dry-run`).
pub fn init(root: &Path, exe: &Path, dry_run: bool) -> Result<Vec<String>> {
    let root = crate::walk::canonical(root)?;
    let harnesses = Harnesses::detect(&root);
    init_for(&root, exe, dry_run, &harnesses)
}

fn init_for(root: &Path, exe: &Path, dry_run: bool, h: &Harnesses) -> Result<Vec<String>> {
    let (exe, root_s) = (exe.display().to_string(), root.display().to_string());
    let serve = json!({ "command": exe, "args": ["serve", "--root", root_s] });
    // Outils différés derrière la recherche d'outils de Claude Code : l'agent ignore lcs.
    let mut claude = serve.clone();
    claude["alwaysLoad"] = json!(true);

    let mut actions = vec![mcp_json(&root.join(".mcp.json"), claude, dry_run)?, md_section(&root.join("CLAUDE.md"), dry_run)?];
    if h.codex {
        actions.push(codex_mcp(&root.join(".codex").join("config.toml"), &exe, &root_s, dry_run)?);
        actions.push(md_section(&root.join("AGENTS.md"), dry_run)?);
    }
    if h.cursor {
        let cursor = root.join(".cursor");
        actions.push(mcp_json(&cursor.join("mcp.json"), serve, dry_run)?);
        let rule = format!(
            "---\ndescription: When to use local-code-search or grep to find code\nalwaysApply: true\n---\n\n{GUIDE}\n"
        );
        let label = "règle « quand utiliser search ou grep »";
        actions.push(write_if_changed(&cursor.join("rules").join("local-code-search.mdc"), &rule, label, dry_run)?);
    }
    if dry_run {
        actions.push("--dry-run : aucun fichier modifié".into());
    }
    Ok(actions)
}

fn read_json(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("JSON invalide : {}", path.display())),
        Err(_) => Ok(json!({})),
    }
}

/// `mcpServers.local-code-search` dans un fichier au format `.mcp.json` (Claude Code, Cursor).
fn mcp_json(path: &Path, server: Value, dry_run: bool) -> Result<String> {
    let mut config = read_json(path)?;
    if config["mcpServers"][SERVER] == server {
        return Ok(format!("{} : déjà à jour", path.display()));
    }
    if !config.is_object() {
        config = json!({});
    }
    if !config["mcpServers"].is_object() {
        config["mcpServers"] = json!({});
    }
    config["mcpServers"][SERVER] = server;
    write(path, &(serde_json::to_string_pretty(&config)? + "\n"), dry_run)?;
    Ok(format!("{} : serveur MCP « {SERVER} » enregistré", path.display()))
}

/// `[mcp_servers.local-code-search]` dans `.codex/config.toml`, modifié comme du texte pour garder les
/// commentaires et l'ordre du fichier.
fn codex_mcp(path: &Path, exe: &str, root: &str, dry_run: bool) -> Result<String> {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    let updated = codex_toml(&current, exe, root);
    if updated == current {
        return Ok(format!("{} : déjà à jour", path.display()));
    }
    toml::from_str::<toml::Table>(&updated).with_context(|| format!("TOML invalide : {}", path.display()))?;
    write(path, &updated, dry_run)?;
    Ok(format!("{} : serveur MCP « {SERVER} » enregistré (projets approuvés seulement)", path.display()))
}

fn codex_toml(current: &str, exe: &str, root: &str) -> String {
    let q = |s: &str| toml::Value::String(s.to_string()).to_string();
    let header = format!("[mcp_servers.{SERVER}]");
    let block = format!("{header}\ncommand = {}\nargs = [{}, {}, {}]", q(exe), q("serve"), q("--root"), q(root));
    let lines: Vec<&str> = current.lines().collect();
    match lines.iter().position(|l| l.trim() == header) {
        Some(start) => {
            // Jusqu'à la table suivante : les sous-tables (`[mcp_servers.local-code-search.env]`) restent.
            let end = lines[start + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |i| start + 1 + i);
            let mut out: Vec<&str> = lines[..start].to_vec();
            out.push(&block);
            if end < lines.len() {
                out.push("");
            }
            out.extend(&lines[end..]);
            out.join("\n") + "\n"
        }
        None if current.trim().is_empty() => format!("{block}\n"),
        None => format!("{}\n\n{block}\n", current.trim_end()),
    }
}

/// Section balisée dans un fichier de consignes, créé s'il n'existe pas.
fn md_section(path: &Path, dry_run: bool) -> Result<String> {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    let section = format!("{BEGIN}\n{GUIDE}\n{END}");
    let updated = match (current.find(BEGIN), current.find(END)) {
        (Some(b), Some(e)) if e > b => format!("{}{section}{}", &current[..b], &current[e + END.len()..]),
        _ if current.is_empty() => format!("{section}\n"),
        _ => format!("{}\n\n{section}\n", current.trim_end()),
    };
    if updated == current {
        return Ok(format!("{} : déjà à jour", path.display()));
    }
    write(path, &updated, dry_run)?;
    Ok(format!("{} : section « quand utiliser search ou grep » ajoutée", path.display()))
}

fn write_if_changed(path: &Path, content: &str, label: &str, dry_run: bool) -> Result<String> {
    if std::fs::read_to_string(path).is_ok_and(|c| c == content) {
        return Ok(format!("{} : déjà à jour", path.display()));
    }
    write(path, content, dry_run)?;
    Ok(format!("{} : {label}", path.display()))
}

fn write(path: &Path, content: &str, dry_run: bool) -> Result<()> {
    if dry_run {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.exists() {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        std::fs::copy(path, bak)?;
    }
    std::fs::write(path, content).with_context(|| format!("écriture impossible : {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lcs-init-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: PathBuf) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn idempotent_et_conserve_le_contenu_existant() {
        let dir = temp("claude");
        std::fs::write(dir.join("CLAUDE.md"), "# Mon projet\n").unwrap();
        std::fs::write(dir.join(".mcp.json"), r#"{"mcpServers":{"autre":{"command":"x"}}}"#).unwrap();
        let (exe, none) = (Path::new("lcs"), Harnesses { codex: false, cursor: false });
        init_for(&dir, exe, false, &none).unwrap();
        let second = init_for(&dir, exe, false, &none).unwrap();
        assert!(second.iter().all(|a| a.contains("déjà à jour")), "{second:?}");
        let mcp: Value = serde_json::from_str(&read(dir.join(".mcp.json"))).unwrap();
        assert!(mcp["mcpServers"]["autre"].is_object() && mcp["mcpServers"][SERVER].is_object());
        assert_eq!(mcp["mcpServers"][SERVER]["alwaysLoad"], json!(true), "outils lcs chargés d'office");
        assert!(mcp["mcpServers"]["autre"].get("alwaysLoad").is_none(), "les autres serveurs ne sont pas touchés");
        let md = read(dir.join("CLAUDE.md"));
        assert!(md.starts_with("# Mon projet") && md.matches(BEGIN).count() == 1);
        assert!(dir.join("CLAUDE.md.bak").exists());
        assert!(!dir.join(".codex").exists() && !dir.join(".cursor").exists() && !dir.join("AGENTS.md").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn codex_et_cursor_branches_sans_toucher_au_reste() {
        let dir = temp("all");
        std::fs::create_dir_all(dir.join(".codex")).unwrap();
        let config = "# mes réglages\nmodel = \"x\"\n\n[mcp_servers.autre]\ncommand = \"y\"\n";
        std::fs::write(dir.join(".codex/config.toml"), config).unwrap();
        std::fs::write(dir.join("AGENTS.md"), "# Consignes\n").unwrap();
        let (exe, all) = (Path::new("C:\\Program Files\\lcs.exe"), Harnesses { codex: true, cursor: true });
        init_for(&dir, exe, false, &all).unwrap();
        let second = init_for(&dir, exe, false, &all).unwrap();
        assert!(second.iter().all(|a| a.contains("déjà à jour")), "{second:?}");

        let toml_text = read(dir.join(".codex/config.toml"));
        assert!(toml_text.starts_with(config.trim_end()), "contenu d'origine gardé tel quel : {toml_text}");
        let parsed: toml::Table = toml::from_str(&toml_text).unwrap();
        let lcs = &parsed["mcp_servers"][SERVER];
        assert_eq!(lcs["command"].as_str(), Some("C:\\Program Files\\lcs.exe"));
        assert_eq!(lcs["args"].as_array().unwrap()[0].as_str(), Some("serve"));
        assert!(parsed["mcp_servers"]["autre"].is_table());

        assert!(read(dir.join("AGENTS.md")).starts_with("# Consignes"));
        let cursor: Value = serde_json::from_str(&read(dir.join(".cursor/mcp.json"))).unwrap();
        assert_eq!(cursor["mcpServers"][SERVER]["args"][0], "serve");
        let rule = read(dir.join(".cursor/rules/local-code-search.mdc"));
        assert!(rule.contains("alwaysApply: true") && rule.contains("`search`"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn codex_remplace_l_ancienne_entree_et_garde_les_sous_tables() {
        let before = "[mcp_servers.local-code-search]\ncommand = \"ancien\"\nargs = []\n\n[mcp_servers.local-code-search.env]\nA = \"1\"\n\n[autre]\nb = 2\n";
        let after = codex_toml(before, "nouveau", "/projet");
        assert_eq!(after.matches("[mcp_servers.local-code-search]").count(), 1, "{after}");
        assert!(after.contains("command = \"nouveau\"") && !after.contains("ancien"), "{after}");
        assert!(after.contains("[mcp_servers.local-code-search.env]\nA = \"1\"") && after.contains("[autre]\nb = 2"), "{after}");
        assert_eq!(codex_toml(&after, "nouveau", "/projet"), after, "deuxième passage sans changement");
        toml::from_str::<toml::Table>(&after).unwrap();
    }
}
