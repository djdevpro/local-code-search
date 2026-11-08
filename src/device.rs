//! Choix du backend matériel : GPU de la plateforme puis CPU, repli toujours annoncé.

use anyhow::{Result, bail};
use fastembed::ExecutionProviderDispatch;

pub struct Candidate {
    pub name: &'static str,
    /// `None` = CPU.
    pub provider: Option<ExecutionProviderDispatch>,
}

fn cpu() -> Candidate {
    Candidate { name: "cpu", provider: None }
}

#[cfg(windows)]
fn gpu(name: &str) -> Option<Candidate> {
    (name == "directml").then(|| Candidate {
        name: "directml",
        provider: Some(ort::ep::DirectML::default().build().error_on_failure()),
    })
}

#[cfg(target_os = "macos")]
fn gpu(name: &str) -> Option<Candidate> {
    (name == "coreml").then(|| Candidate {
        name: "coreml",
        provider: Some(ort::ep::CoreML::default().build().error_on_failure()),
    })
}

#[cfg(target_os = "linux")]
fn gpu(name: &str) -> Option<Candidate> {
    (name == "cuda").then(|| Candidate {
        name: "cuda",
        provider: Some(ort::ep::CUDA::default().build().error_on_failure()),
    })
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn gpu(_name: &str) -> Option<Candidate> {
    None
}

/// Le GPU natif de cette plateforme.
pub const NATIVE_GPU: &str = if cfg!(windows) {
    "directml"
} else if cfg!(target_os = "macos") {
    "coreml"
} else {
    "cuda"
};

fn candidates(device: &str) -> Result<Vec<Candidate>> {
    Ok(match device {
        "cpu" => vec![cpu()],
        // macOS : CoreML recompile le modèle à chaque longueur de texte (formes dynamiques) et s'est
        // montré très lent en CI (plus d'une heure contre 5 min sur CPU ailleurs). Tant qu'il n'est
        // pas validé sur un vrai Mac, il n'est utilisé que sur demande explicite (--device coreml).
        "auto" if cfg!(target_os = "macos") => vec![cpu()],
        "auto" => gpu(NATIVE_GPU).into_iter().chain([cpu()]).collect(),
        other => match gpu(other) {
            Some(c) => vec![c],
            None => bail!("le backend « {other} » n'existe pas sur cette plateforme (disponible : {NATIVE_GPU}, cpu)"),
        },
    })
}

/// Essaie chaque backend dans l'ordre ; renvoie (résultat, backend utilisé, notes de repli).
/// Un backend demandé explicitement qui échoue est une erreur, jamais un repli silencieux.
pub fn try_each<T>(device: &str, mut build: impl FnMut(Option<ExecutionProviderDispatch>) -> Result<T>) -> Result<(T, &'static str, Vec<String>)> {
    let mut notes = Vec::new();
    let mut last_error = None;
    for c in candidates(device)? {
        let name = c.name;
        match build(c.provider) {
            Ok(v) => return Ok((v, name, notes)),
            Err(e) if device == "auto" && name != "cpu" => notes.push(format!("{name} indisponible ({e:#}) : repli sur le CPU")),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("aucun backend utilisable")))
}
