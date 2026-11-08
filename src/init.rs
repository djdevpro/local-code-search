//! Branche l'outil dans les agents de code :
//! - Claude Code (toujours) : `.mcp.json`, hook `UserPromptSubmit` (`.claude/settings.json`), section CLAUDE.md ;
//! - Codex (`.codex/` dans le projet, `~/.codex` ou `CODEX_HOME`) : `.codex/config.toml`, hook
//!   `UserPromptSubmit` (`.codex/hooks.json`, même format que Claude Code), section AGENTS.md ;
//! - Cursor (`.cursor/` dans le projet ou `~/.cursor`) : `.cursor/mcp.json` et une règle toujours appliquée
//!   (`.cursor/rules/local-code-search.mdc`). Pas de hook : `beforeSubmitPrompt` ne peut pas ajouter de
//!   contexte (seulement `continue` et `user_message`) et `sessionStart` part avant la demande.
//!
//! Sauvegarde `.bak`, mode `--dry-run`, aucun secret, aucune consigne d'envoi de données.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SERVER: &str = "local-code-search";
const BEGIN: &str = "<!-- local-code-search:début -->";
const END: &str = "<!-- local-code-search:fin -->";

/// Consignes communes (CLAUDE.md, AGENTS.md, règle Cursor).
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

/// Seulement là où le hook existe (Claude Code, Codex).
const HOOK_LINE: &str =
    "- When a prompt comes with \"Files probably involved\" (added by local-code-search), start from those files.";

/// Agents de code à brancher en plus de Claude Code.
pub struct Harnesses {
    pub codex: bool,
    pub cursor: bool,
}

impl Harnesses {
    /// Présents dans le projet (`.codex/`, `.cursor/`) ou installés sur la machine (`~/.codex`, `~/.cursor`).
    fn detect(root: &Path) -> Self {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
        let present = |name: &str| root.join(name).is_dir() || home.as_ref().is_some_and(|h| h.join(name).is_dir());
        let codex_home = std::env::var_os("CODEX_HOME").is_some_and(|p| Path::new(&p).is_dir());
        Harnesses { codex: present(".codex") || codex_home, cursor: present(".cursor") }
    }
}

/// Renvoie la liste des actions (effectuées, ou prévues en `--dry-run`).
/// `hook` : pose aussi le hook « fichiers probablement concernés » (`[hook] enabled`, faux par défaut).
pub fn init(root: &Path, exe: &Path, dry_run: bool, hook: bool) -> Result<Vec<String>> {
    let root = crate::walk::canonical(root)?;
    let harnesses = Harnesses::detect(&root);
    init_for(&root, exe, dry_run, &harnesses, hook)
}

fn init_for(root: &Path, exe: &Path, dry_run: bool, h: &Harnesses, with_hook: bool) -> Result<Vec<String>> {
    let (exe, root_s) = (exe.display().to_string(), root.display().to_string());
    let serve = json!({ "command": exe, "args": ["serve", "--root", root_s] });
    // Claude Code : outils chargés d'office pour ce seul serveur (`alwaysLoad`), les autres serveurs
    // gardent leur chargement à la demande. Mesuré : outils différés → l'agent ignore lcs.
    let mut claude = serve.clone();
    claude["alwaysLoad"] = json!(true);
    // Chemins entre guillemets : lancé par bash (Claude Code), `$SHELL -lc` ou `cmd.exe /C ""…""` (Codex,
    // qui ajoute lui-même les guillemets extérieurs sous Windows).
    let hook = format!("\"{exe}\" hook prompt --root \"{root_s}\"");
    let guide = if with_hook { format!("{GUIDE}\n{HOOK_LINE}") } else { GUIDE.to_string() };

    let mut actions = vec![mcp_json(&root.join(".mcp.json"), claude, dry_run)?];
    if with_hook {
        actions.push(prompt_hook(&root.join(".claude").join("settings.json"), &hook, "", dry_run)?);
    }
    actions.push(md_section(&root.join("CLAUDE.md"), &guide, dry_run)?);
    if h.codex {
        let codex = root.join(".codex");
        actions.push(codex_mcp(&codex.join("config.toml"), &exe, &root_s, dry_run)?);
        if with_hook {
            let note = " (Codex : projet à approuver, puis hook à valider une fois avec /hooks)";
            actions.push(prompt_hook(&codex.join("hooks.json"), &hook, note, dry_run)?);
        }
        actions.push(md_section(&root.join("AGENTS.md"), &guide, dry_run)?);
    }
    if h.cursor {
        let cursor = root.join(".cursor");
        actions.push(mcp_json(&cursor.join("mcp.json"), serve, dry_run)?);
        let rule = format!(
            "---\ndescription: When to use local-code-search or grep to find code\nalwaysApply: true\n---\n\n{GUIDE}\n"
        );
        let label = "règle « quand utiliser search ou grep » (Cursor n'a pas de hook capable d'ajouter du contexte)";
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

/// Hook `UserPromptSubmit` (`hooks.UserPromptSubmit`, format commun à Claude Code et Codex) : avant le
/// premier tour, l'agent reçoit les fichiers probablement concernés par la demande. Les autres hooks et
/// réglages sont conservés ; une ancienne entrée lcs (autre chemin de binaire) est remplacée.
fn prompt_hook(path: &Path, command: &str, note: &str, dry_run: bool) -> Result<String> {
    let mut settings = read_json(path)?;
    let entry = json!({ "hooks": [{ "type": "command", "command": command, "timeout": 30 }] });
    let is_lcs = |e: &Value| {
        e["hooks"].as_array().is_some_and(|hs| hs.iter().any(|h| h["command"].as_str().is_some_and(|c| c.contains(" hook prompt"))))
    };
    let list = settings["hooks"]["UserPromptSubmit"].as_array().cloned().unwrap_or_default();
    if list.iter().any(|e| e == &entry) {
        return Ok(format!("{} : déjà à jour", path.display()));
    }
    let mut list: Vec<Value> = list.into_iter().filter(|e| !is_lcs(e)).collect();
    list.push(entry);
    if !settings.is_object() {
        settings = json!({});
    }
    if !settings["hooks"].is_object() {
        settings["hooks"] = json!({});
    }
    settings["hooks"]["UserPromptSubmit"] = Value::Array(list);
    write(path, &(serde_json::to_string_pretty(&settings)? + "\n"), dry_run)?;
    Ok(format!("{} : hook « fichiers probablement concernés » enregistré{note}", path.display()))
}

/// `[mcp_servers.local-code-search]` dans `.codex/config.toml`, modifié comme du texte : les autres
/// tables, l'ordre et les commentaires du fichier restent tels quels.
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
            // Le bloc s'arrête à la table suivante ; ses sous-tables (`[mcp_servers.local-code-search.env]`) restent.
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

/// Section balisée dans un fichier de consignes (CLAUDE.md, AGENTS.md), créé s'il n'existe pas.
fn md_section(path: &Path, guide: &str, dry_run: bool) -> Result<String> {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    let section = format!("{BEGIN}\n{guide}\n{END}");
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

/// Écrit (sauf en `--dry-run`) après une sauvegarde `.bak` du fichier existant.
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
        init_for(&dir, exe, false, &none, true).unwrap();
        let second = init_for(&dir, exe, false, &none, true).unwrap();
        assert!(second.iter().filter(|a| a.contains("déjà à jour")).count() == 3, "{second:?}");
        let settings: Value = serde_json::from_str(&read(dir.join(".claude/settings.json"))).unwrap();
        let hooks = settings["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(hooks.len(), 1, "une seule entrée lcs, même après deux init");
        assert!(hooks[0]["hooks"][0]["command"].as_str().unwrap().contains("hook prompt --root"));
        let mcp: Value = serde_json::from_str(&read(dir.join(".mcp.json"))).unwrap();
        assert!(mcp["mcpServers"]["autre"].is_object() && mcp["mcpServers"][SERVER].is_object());
        assert_eq!(mcp["mcpServers"][SERVER]["alwaysLoad"], json!(true), "outils lcs chargés d'office");
        assert!(mcp["mcpServers"]["autre"].get("alwaysLoad").is_none(), "les autres serveurs ne sont pas touchés");
        let md = read(dir.join("CLAUDE.md"));
        assert!(md.starts_with("# Mon projet") && md.matches(BEGIN).count() == 1 && md.contains(HOOK_LINE));
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
        std::fs::write(dir.join(".codex/hooks.json"), r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"z"}]}]}}"#).unwrap();
        std::fs::write(dir.join("AGENTS.md"), "# Consignes\n").unwrap();
        let (exe, all) = (Path::new("C:\\Program Files\\lcs.exe"), Harnesses { codex: true, cursor: true });
        init_for(&dir, exe, false, &all, true).unwrap();
        let second = init_for(&dir, exe, false, &all, true).unwrap();
        assert!(second.iter().all(|a| a.contains("déjà à jour")), "{second:?}");

        let toml_text = read(dir.join(".codex/config.toml"));
        assert!(toml_text.starts_with(config.trim_end()), "contenu d'origine gardé tel quel : {toml_text}");
        let parsed: toml::Table = toml::from_str(&toml_text).unwrap();
        let lcs = &parsed["mcp_servers"][SERVER];
        assert_eq!(lcs["command"].as_str(), Some("C:\\Program Files\\lcs.exe"));
        assert_eq!(lcs["args"].as_array().unwrap()[0].as_str(), Some("serve"));
        assert!(parsed["mcp_servers"]["autre"].is_table());

        let hooks: Value = serde_json::from_str(&read(dir.join(".codex/hooks.json"))).unwrap();
        assert_eq!(hooks["hooks"]["Stop"][0]["hooks"][0]["command"], "z", "les autres hooks restent");
        let cmd = hooks["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.starts_with("\"C:\\Program Files\\lcs.exe\" hook prompt --root \""), "{cmd}");

        assert!(read(dir.join("AGENTS.md")).starts_with("# Consignes") && read(dir.join("AGENTS.md")).contains(HOOK_LINE));
        let cursor: Value = serde_json::from_str(&read(dir.join(".cursor/mcp.json"))).unwrap();
        assert_eq!(cursor["mcpServers"][SERVER]["args"][0], "serve");
        let rule = read(dir.join(".cursor/rules/local-code-search.mdc"));
        assert!(rule.contains("alwaysApply: true") && rule.contains("`search`") && !rule.contains(HOOK_LINE));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sans_hook_par_defaut() {
        let dir = temp("nohook");
        std::fs::create_dir_all(dir.join(".codex")).unwrap();
        let actions = init_for(&dir, Path::new("lcs"), false, &Harnesses { codex: true, cursor: false }, false).unwrap();
        assert!(!actions.iter().any(|a| a.contains("fichiers probablement concernés")), "{actions:?}");
        assert!(!dir.join(".claude/settings.json").exists() && !dir.join(".codex/hooks.json").exists());
        assert!(dir.join(".codex/config.toml").exists(), "le serveur MCP reste enregistré");
        assert!(!read(dir.join("CLAUDE.md")).contains(HOOK_LINE) && !read(dir.join("AGENTS.md")).contains(HOOK_LINE));
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
