//! Inventaire des connexions sortantes : échoue si une nouvelle apparaît dans le code.
//! Exceptions autorisées, toutes sur commande explicite : le téléchargement d'un modèle
//! (`embed.rs`, hf-hub) et `lcs update` (`update.rs`, API GitHub des versions).

use std::fs;

const ALLOWED: &[(&str, &str)] = &[
    ("src/embed.rs", "hf_hub::api::sync"),
    ("src/update.rs", "https://api.github.com/repos/"),
    ("src/update.rs", "https://github.com/"),
    ("src/update.rs", "ureq::"),
];
const SUSPECT: &[&str] = &[
    "http://", "https://", "hf_hub", "reqwest", "ureq", "TcpStream", "UdpSocket", "std::net", "tokio::net", "hyper",
];

#[test]
fn aucune_connexion_sortante_non_declaree() {
    let mut violations = Vec::new();
    for entry in fs::read_dir("src").expect("dossier src") {
        let path = entry.expect("entrée").path();
        let rel = format!("src/{}", path.file_name().unwrap().to_string_lossy());
        let text = fs::read_to_string(&path).expect("lecture");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for s in SUSPECT.iter().filter(|s| code.contains(*s)) {
                let allowed = ALLOWED.iter().any(|(f, pat)| *f == rel && code.contains(pat));
                if !allowed {
                    violations.push(format!("{rel}:{} contient « {s} »", n + 1));
                }
            }
        }
    }
    assert!(violations.is_empty(), "connexions sortantes non déclarées :\n{}", violations.join("\n"));
}

#[test]
fn dependances_reseau_connues() {
    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo.toml");
    for forbidden in ["reqwest", "hyper", "surf", "isahc"] {
        assert!(!manifest.contains(forbidden), "dépendance réseau directe interdite : {forbidden}");
    }
}
