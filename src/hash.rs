//! Détection des changements : (taille, date) filtrent, le hash blake3 du contenu confirme.

use crate::types::{FileEntry, FileState};
use std::collections::HashMap;

/// Ce qu'il faut faire pour mettre l'index à jour.
#[derive(Debug, Default)]
pub struct Plan {
    /// Fichiers nouveaux ou modifiés, avec leur hash.
    pub changed: Vec<(FileEntry, String)>,
    /// Contenu identique mais date différente : seul le manifeste change.
    pub touched: Vec<(FileEntry, String)>,
    /// Fichiers disparus.
    pub deleted: Vec<String>,
    pub unchanged: usize,
}

pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn plan(manifest: &HashMap<String, FileState>, files: Vec<FileEntry>) -> Plan {
    let mut plan = Plan::default();
    let present: std::collections::HashSet<&str> = files.iter().map(|f| f.rel.as_str()).collect();
    plan.deleted = manifest.keys().filter(|p| !present.contains(p.as_str())).cloned().collect();
    plan.deleted.sort();

    for file in files {
        let known = manifest.get(&file.rel);
        if known.is_some_and(|k| k.size == file.size && k.mtime == file.mtime) {
            plan.unchanged += 1;
            continue;
        }
        let Ok(bytes) = std::fs::read(&file.abs) else { continue };
        let hash = content_hash(&bytes);
        match known {
            Some(k) if k.hash == hash => plan.touched.push((file, hash)),
            _ => plan.changed.push((file, hash)),
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecte_suppressions_et_inchanges() {
        let dir = std::env::temp_dir().join(format!("lcs-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.rs");
        std::fs::write(&path, "fn a() {}").unwrap();
        let entry = FileEntry { abs: path, rel: "a.rs".into(), size: 9, mtime: 1 };
        let mut manifest = HashMap::new();
        manifest.insert("a.rs".into(), FileState { size: 9, mtime: 1, hash: "x".into() });
        manifest.insert("gone.rs".into(), FileState { size: 1, mtime: 1, hash: "y".into() });

        let p = plan(&manifest, vec![entry.clone()]);
        assert_eq!((p.unchanged, p.deleted.clone()), (1, vec!["gone.rs".to_string()]));

        let moved = FileEntry { mtime: 2, ..entry };
        let p = plan(&manifest, vec![moved]);
        assert_eq!(p.changed.len(), 1, "hash différent => modifié");
        std::fs::remove_dir_all(dir).ok();
    }
}
