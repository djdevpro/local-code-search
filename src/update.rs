//! `lcs update` : passe à la dernière version publiée sur GitHub (somme SHA-256 vérifiée).
//! Seule connexion sortante avec le téléchargement des modèles, et seulement sur commande.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

const REPO: &str = "djdevpro/local-code-search";
const MAX_DOWNLOAD: u64 = 300 << 20;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Le fichier publié par le workflow Release pour ce système.
fn asset_name() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("lcs-windows-x64.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("lcs-linux-x64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("lcs-macos-arm64")
    } else {
        None
    }
}

/// `v1.2.3` ou `1.2.3-rc.1` en nombres comparables.
fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim_start_matches('v').split(['.', '-', '+']).map(|p| p.parse().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// La somme attendue pour `name` dans un fichier au format de `sha256sum`.
fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim_start_matches([' ', '*']) == name).then(|| hash.to_lowercase())
    })
}

fn agent(current: &str) -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("local-code-search/{current}"))
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into()
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    Ok(agent.get(url).call()?.body_mut().with_config().limit(MAX_DOWNLOAD).read_to_vec()?)
}

pub fn run(current: &str, check_only: bool) -> Result<()> {
    let agent = agent(current);
    let release: Release = agent
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .with_context(|| format!("impossible de lire la dernière version publiée sur github.com/{REPO}"))?
        .body_mut()
        .read_json()?;

    let latest = release.tag_name.trim_start_matches('v');
    let (Some(new), Some(cur)) = (parse(latest), parse(current)) else {
        bail!("version illisible ({} ou {current})", release.tag_name);
    };
    if new <= cur {
        println!("lcs {current} est déjà la dernière version.");
        return Ok(());
    }
    println!("lcs {latest} est sorti, tu as la {current}.");
    if check_only {
        println!("lcs update pour l'installer.");
        return Ok(());
    }

    let name = asset_name()
        .with_context(|| format!("aucun binaire publié pour ce système : cargo install --git https://github.com/{REPO}"))?;
    let url = |n: &str| release.assets.iter().find(|a| a.name == n).map(|a| a.browser_download_url.as_str());
    let bin_url = url(name).with_context(|| format!("{name} manque dans la version {latest}"))?;
    let sums_url = url("SHA256SUMS.txt").with_context(|| format!("SHA256SUMS.txt manque dans la version {latest}"))?;

    let bin = download(&agent, bin_url)?;
    let sums = String::from_utf8(download(&agent, sums_url)?)?;
    let expected = expected_sum(&sums, name).with_context(|| format!("pas de somme de contrôle pour {name}"))?;
    let got: String = Sha256::digest(&bin).iter().map(|b| format!("{b:02x}")).collect();
    if got != expected {
        bail!("le fichier téléchargé ne correspond pas à sa somme de contrôle, rien n'a changé");
    }

    // Sous Windows, l'exe en cours d'exécution est verrouillé : self_replace le met de côté,
    // installe le nouveau à sa place et supprime l'ancien à la sortie.
    let tmp = std::env::temp_dir().join(format!("lcs-{latest}-{}", std::process::id()));
    std::fs::write(&tmp, &bin)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    let replaced = self_replace::self_replace(&tmp);
    let _ = std::fs::remove_file(&tmp);
    replaced.context("impossible de remplacer lcs, vérifie les droits sur son dossier")?;
    println!("lcs est passé en version {latest}.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_et_sommes() {
        assert!(parse("v0.10.0") > parse("0.9.3"), "comparaison en nombres, pas en texte");
        assert_eq!(parse("v1.2.3-rc.1"), Some((1, 2, 3)));
        assert_eq!(parse("v1.2"), None);
        let sums = "aaa111  lcs-linux-x64\nBBB222 *lcs-windows-x64.exe\n";
        assert_eq!(expected_sum(sums, "lcs-windows-x64.exe").as_deref(), Some("bbb222"));
        assert_eq!(expected_sum(sums, "lcs-linux-x64").as_deref(), Some("aaa111"));
        assert_eq!(expected_sum(sums, "lcs-macos-arm64"), None);
    }
}
