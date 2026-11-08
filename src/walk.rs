//! Parcours des fichiers : `.gitignore` (imbriqués), exclusions par défaut et de la config,
//! fichiers binaires et minifiés écartés.

use crate::config::Config;
use crate::types::FileEntry;
use anyhow::Result;
use ignore::{WalkBuilder, overrides::OverrideBuilder};
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

const DEFAULT_EXCLUDES: [&str; 9] = [
    "node_modules", "target", "dist", "build", "vendor", ".venv", "venv", "__pycache__", ".git",
];
/// Au-delà, le fichier n'est pas du code source écrit à la main.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Fichiers écartés, pour les annoncer (jamais d'exclusion silencieuse).
#[derive(Debug, Default)]
pub struct Skipped {
    pub binary: usize,
    pub minified: usize,
    pub too_large: usize,
}

pub fn files(root: &Path, config: &Config) -> Result<(Vec<FileEntry>, Skipped)> {
    let mut overrides = OverrideBuilder::new(root);
    for pattern in DEFAULT_EXCLUDES.iter().map(|d| format!("**/{d}/**")) {
        overrides.add(&format!("!{pattern}"))?;
    }
    overrides.add(&format!("!{}/**", config.index.data_dir))?;
    for pattern in &config.index.exclude {
        overrides.add(&format!("!{pattern}"))?;
    }
    let walker = WalkBuilder::new(root)
        .overrides(overrides.build()?)
        .require_git(false)
        .build();

    let mut entries = Vec::new();
    let mut skipped = Skipped::default();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.len() > MAX_FILE_BYTES {
            skipped.too_large += 1;
            continue;
        }
        match sniff(entry.path()) {
            Sniff::Binary => skipped.binary += 1,
            Sniff::Minified => skipped.minified += 1,
            Sniff::Text => entries.push(FileEntry {
                abs: entry.path().to_path_buf(),
                rel: relative(root, entry.path()),
                size: meta.len(),
                mtime: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos() as u64),
            }),
        }
    }
    entries.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok((entries, skipped))
}

/// Chemin absolu sans le préfixe `\\?\` de Windows.
pub fn canonical(path: &Path) -> Result<std::path::PathBuf> {
    let abs = path.canonicalize().map_err(|e| anyhow::anyhow!("dossier introuvable : {} ({e})", path.display()))?;
    let text = abs.to_string_lossy();
    Ok(match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => std::path::PathBuf::from(rest),
        _ => abs,
    })
}

pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

enum Sniff {
    Text,
    Binary,
    Minified,
}

fn sniff(path: &Path) -> Sniff {
    let mut head = vec![0u8; 8192];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.contains(&0) {
        return Sniff::Binary;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let minifiable = matches!(ext.as_str(), "js" | "mjs" | "cjs" | "css" | "json" | "map" | "html" | "svg");
    let lines = head.split(|&b| b == b'\n').count().max(1);
    if minifiable && n >= 4096 && n / lines > 500 { Sniff::Minified } else { Sniff::Text }
}
