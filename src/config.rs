use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FILE_NAME: &str = "local-code-search.toml";
const DEVICES: [&str; 5] = ["auto", "cpu", "cuda", "directml", "coreml"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub model: Model,
    pub reranker: Reranker,
    pub device: String,
    pub index: Index,
    pub search: Search,
    pub output: Output,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Model {
    /// Identifiant Hugging Face ou dossier local.
    pub embedder: String,
    /// Variante ONNX utilisée sur CPU (quantifiée int8).
    pub onnx_file: String,
    /// Variante ONNX utilisée sur GPU (fp32 : le fp16 de gte-modernbert sort des zéros sur DirectML).
    pub onnx_file_gpu: String,
    pub query_prefix: String,
    pub document_prefix: String,
    pub max_tokens: usize,
    pub batch_size: usize,
    /// Threads de calcul sur CPU (0 = automatique).
    pub cpu_threads: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Reranker {
    pub model: String,
    pub onnx_file: String,
    pub onnx_file_gpu: String,
    pub enabled: bool,
    pub candidates: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Index {
    pub data_dir: String,
    pub languages: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Search {
    pub rrf_k: f32,
    pub bm25_weight: f32,
    pub vector_weight: f32,
    pub results: usize,
    pub test_penalty: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Output {
    pub token_budget: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: Model::default(),
            reranker: Reranker::default(),
            device: "auto".into(),
            index: Index::default(),
            search: Search::default(),
            output: Output::default(),
        }
    }
}

impl Default for Model {
    fn default() -> Self {
        Self {
            embedder: "Alibaba-NLP/gte-modernbert-base".into(),
            onnx_file: "onnx/model_int8.onnx".into(),
            onnx_file_gpu: "onnx/model.onnx".into(),
            query_prefix: String::new(),
            document_prefix: String::new(),
            max_tokens: 512,
            batch_size: 32,
            cpu_threads: 0,
        }
    }
}

impl Default for Reranker {
    fn default() -> Self {
        Self {
            model: "Alibaba-NLP/gte-reranker-modernbert-base".into(),
            onnx_file: "onnx/model_int8.onnx".into(),
            onnx_file_gpu: "onnx/model.onnx".into(),
            enabled: false,
            candidates: 50,
        }
    }
}

impl Default for Index {
    fn default() -> Self {
        Self {
            data_dir: ".lcs".into(),
            languages: ["python", "typescript", "javascript", "rust", "go", "php"].map(String::from).into(),
            exclude: Vec::new(),
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Self { rrf_k: 60.0, bm25_weight: 0.3, vector_weight: 1.0, results: 10, test_penalty: 0.5 }
    }
}

impl Default for Output {
    fn default() -> Self {
        Self { token_budget: 1500 }
    }
}

/// Charge `<root>/local-code-search.toml` s'il existe, sinon les valeurs par défaut,
/// puis applique l'option `--device` de la ligne de commande.
pub fn load(root: &Path, device: Option<String>) -> Result<Config> {
    let path = root.join(FILE_NAME);
    let mut config: Config = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("config invalide : {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(e).with_context(|| format!("lecture impossible : {}", path.display())),
    };
    if let Some(d) = device {
        config.device = d;
    }
    validate(config)
}

fn validate(config: Config) -> Result<Config> {
    if !DEVICES.contains(&config.device.as_str()) {
        bail!("device inconnu « {} » (attendu : {})", config.device, DEVICES.join(" | "));
    }
    if config.model.batch_size == 0 || config.search.results == 0 {
        bail!("batch_size et results doivent être supérieurs à 0");
    }
    Ok(config)
}
