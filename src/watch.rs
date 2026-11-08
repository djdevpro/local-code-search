//! Surveillance des fichiers avec anti-rebond de 400 ms ; ignore le dossier des données.

use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(400);

/// Bloque et appelle `on_change` après chaque rafale de modifications pertinentes.
pub fn watch(root: &Path, data_dir: &str, mut on_change: impl FnMut()) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut debouncer = new_debouncer(DEBOUNCE, None, tx)?;
    debouncer.watch(root, RecursiveMode::Recursive)?;
    let data = root.join(data_dir);
    for result in rx {
        let Ok(events) = result else { continue };
        let relevant = events
            .iter()
            .flat_map(|e| e.paths.iter())
            .any(|p| !p.starts_with(&data) && !p.components().any(|c| c.as_os_str() == ".git"));
        if relevant {
            on_change();
        }
    }
    Ok(())
}
