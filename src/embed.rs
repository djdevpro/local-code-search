//! Embeddings : modèle Hugging Face ONNX embarqué (fastembed/ort), lots triés par longueur,
//! identité d'embedding (on ne mélange jamais deux espaces vectoriels).

use crate::config::Config;
use crate::device;
use anyhow::{Context, Result};
use fastembed::{
    AutoDevicePolicy, InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use std::path::{Path, PathBuf};

/// Fichiers d'un modèle (tokenizer, pooling), depuis un dossier local ou le cache HF.
pub struct ModelFiles {
    pub tokenizer: TokenizerFiles,
    pub pooling: Pooling,
}

/// Lit la variante ONNX du backend (GPU ou CPU).
pub fn onnx_for(model: &str, provider_is_gpu: bool, cpu_file: &str, gpu_file: &str) -> Result<Vec<u8>> {
    let file = if provider_is_gpu { gpu_file } else { cpu_file };
    Ok(std::fs::read(fetch_file(model, file)?)?)
}

/// Identité d'embedding : poids + tokenizer + pooling + préfixes.
pub fn identity(onnx: &[u8], files: &ModelFiles, extra: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(onnx).update(&files.tokenizer.tokenizer_file);
    h.update(format!("{:?}{extra}", files.pooling).as_bytes());
    h.finalize().to_hex()[..32].to_string()
}

/// Un fichier du modèle : dossier local, sinon téléchargé une fois depuis Hugging Face.
fn fetch_file(model: &str, file: &str) -> Result<PathBuf> {
    let local = Path::new(model);
    if local.is_dir() {
        let p = local.join(file);
        return p.exists().then_some(p).with_context(|| format!("{file} absent de {model}"));
    }
    let api = hf_hub::api::sync::ApiBuilder::from_env().with_progress(true).build()?;
    api.model(model.to_string()).get(file).with_context(|| format!("téléchargement de {model}/{file}"))
}

pub fn fetch(model: &str) -> Result<ModelFiles> {
    let read = |f: &str| -> Result<Vec<u8>> { Ok(std::fs::read(fetch_file(model, f)?)?) };
    let tokenizer = TokenizerFiles {
        tokenizer_file: read("tokenizer.json")?,
        config_file: read("config.json")?,
        special_tokens_map_file: read("special_tokens_map.json")?,
        tokenizer_config_file: read("tokenizer_config.json")?,
    };
    let pooling = pooling_of(read("1_Pooling/config.json").ok().as_deref());
    Ok(ModelFiles { tokenizer, pooling })
}

/// Pooling déclaré par `1_Pooling/config.json` (sentence-transformers) : CLS si
/// `pooling_mode_cls_token` est vrai, moyenne sinon ou sans fichier.
fn pooling_of(config: Option<&[u8]>) -> Pooling {
    let cls = config
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        .and_then(|v| v.get("pooling_mode_cls_token").and_then(serde_json::Value::as_bool))
        .unwrap_or(false);
    if cls { Pooling::Cls } else { Pooling::Mean }
}

/// Threads de la session des questions : 0 = tous les cœurs (mesuré sur SciFact :
/// p95 211 ms avec 1 thread, 130 ms avec 2, 86 ms avec 8, 79 ms avec tous).
const QUERY_THREADS: usize = 0;

pub struct Embedder {
    model: TextEmbedding,
    /// Session CPU avec les mêmes poids, pour les questions quand les documents sont sur GPU :
    /// un GPU se reconfigure à chaque longueur de texte (≈ 200 ms), le CPU répond en ≈ 20 ms.
    /// Créée à la première question seulement (l'indexation n'en a pas besoin).
    query_model: Option<TextEmbedding>,
    files: ModelFiles,
    config: crate::config::Model,
    tokenizer: Option<tokenizers::Tokenizer>,
    pub identity: String,
    pub backend: &'static str,
    pub notes: Vec<String>,
    pub dim: usize,
}

fn session(files: &ModelFiles, m: &crate::config::Model, onnx: Vec<u8>, provider: Option<fastembed::ExecutionProviderDispatch>, threads: usize) -> Result<TextEmbedding> {
    let user = UserDefinedEmbeddingModel::new(onnx, files.tokenizer.clone()).with_pooling(files.pooling.clone());
    let mut options = InitOptionsUserDefined::new().with_max_length(m.max_tokens);
    if threads > 0 {
        options = options.with_intra_threads(threads);
    }
    let options = options
        .with_auto_device(AutoDevicePolicy::PreferCPU)
        .with_execution_providers(provider.into_iter().collect());
    let mut model = TextEmbedding::try_new_from_user_defined(user, options).map_err(|e| anyhow::anyhow!("{e}"))?;
    let probe = model.embed(["fn probe() {}"], None).map_err(|e| anyhow::anyhow!("{e}"))?;
    let valid = probe[0].iter().all(|x| x.is_finite()) && probe[0].iter().any(|x| *x != 0.0);
    anyhow::ensure!(valid, "le modèle produit des vecteurs invalides sur ce backend");
    Ok(model)
}

pub fn load(config: &Config) -> Result<Embedder> {
    let m = &config.model;
    let files = fetch(&m.embedder)?;
    let tokenizer = tokenizers::Tokenizer::from_bytes(&files.tokenizer.tokenizer_file).ok();
    let extra = format!("{}|{}|{}", m.query_prefix, m.document_prefix, m.max_tokens);
    let ((mut model, identity), backend, notes) = device::try_each(&config.device, |provider| {
        let onnx = onnx_for(&m.embedder, provider.is_some(), &m.onnx_file, &m.onnx_file_gpu)?;
        let identity = identity(&onnx, &files, &extra);
        // Sur GPU, le CPU ne fait qu'orchestrer : un seul thread suffit.
        let threads = if provider.is_some() { 1 } else { m.cpu_threads };
        Ok((session(&files, m, onnx, provider, threads)?, identity))
    })?;
    let dim = model.embed(["dimension"], None).map_err(|e| anyhow::anyhow!("{e}"))?[0].len();
    Ok(Embedder { model, query_model: None, files, config: m.clone(), tokenizer, identity, backend, notes, dim })
}

impl Embedder {
    /// Embeddings des documents, calculés par lots de longueurs voisines, ordre d'origine conservé.
    pub fn embed_documents(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut order: Vec<usize> = (0..texts.len()).collect();
        order.sort_by_key(|&i| texts[i].len());
        let sorted: Vec<String> = order.iter().map(|&i| format!("{}{}", self.config.document_prefix, texts[i])).collect();
        let vectors = self.model.embed(&sorted, Some(self.config.batch_size)).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut out = vec![Vec::new(); texts.len()];
        for (v, &i) in vectors.into_iter().zip(&order) {
            out[i] = normalized(v);
        }
        Ok(out)
    }

    pub fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        let text = format!("{}{query}", self.config.query_prefix);
        if self.backend != "cpu" && self.query_model.is_none() {
            let m = &self.config;
            let onnx = onnx_for(&m.embedder, true, &m.onnx_file, &m.onnx_file_gpu)?;
            self.query_model = Some(session(&self.files, m, onnx, None, QUERY_THREADS)?);
        }
        let model = self.query_model.as_mut().unwrap_or(&mut self.model);
        let v = model.embed([text], None).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(normalized(v.into_iter().next().unwrap_or_default()))
    }

    /// Nombre de vrais tokens (tokenizer du modèle), estimation sinon.
    pub fn count_tokens(&self, text: &str) -> usize {
        match &self.tokenizer {
            Some(t) => t.encode(text, false).map_or_else(|_| crate::format::approx_tokens(text), |e| e.len()),
            None => crate::format::approx_tokens(text),
        }
    }
}

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooling_lu_dans_le_json_avec_ou_sans_espaces() {
        assert!(matches!(pooling_of(Some(br#"{"pooling_mode_cls_token": true, "pooling_mode_mean_tokens": false}"#)), Pooling::Cls));
        assert!(matches!(pooling_of(Some(br#"{"pooling_mode_cls_token":true,"pooling_mode_mean_tokens":false}"#)), Pooling::Cls));
        assert!(matches!(pooling_of(Some(br#"{"pooling_mode_cls_token":false,"pooling_mode_mean_tokens":true}"#)), Pooling::Mean));
        assert!(matches!(pooling_of(None), Pooling::Mean), "sans fichier : moyenne");
        assert!(matches!(pooling_of(Some(b"pas du json")), Pooling::Mean));
    }

    /// Même texte, même vecteur quel que soit le backend (télécharge le modèle : `--ignored`).
    #[test]
    #[ignore]
    fn parite_cpu_gpu() {
        let texts = ["fn reciprocal_rank_fusion(legs: Vec<Hits>) -> Vec<Hit>".to_string(), "class UserRepository: pass".to_string()];
        let mut vectors = Vec::new();
        for device in ["cpu", device::NATIVE_GPU] {
            let mut config = Config::default();
            config.device = device.into();
            config.model.onnx_file = config.model.onnx_file_gpu.clone();
            let Ok(mut e) = load(&config) else { continue };
            let v = e.embed_documents(&texts).unwrap();
            println!("{device} ({}) : {:?}", e.backend, &v[0][..4]);
            assert!(v[0].iter().all(|x| x.is_finite()) && v[0].iter().any(|x| *x != 0.0), "{device} : vecteur invalide");
            vectors.push(v);
        }
        if let [a, b] = &vectors[..] {
            let cos: f32 = a[0].iter().zip(&b[0]).map(|(x, y)| x * y).sum();
            println!("cosinus cpu/gpu = {cos}");
            assert!(cos > 0.99, "cosinus {cos}");
        }
    }

    /// Débit sur CPU : `LCS_BENCH_DIR` (fichiers texte), `LCS_THREADS`, `LCS_BATCH` (`--ignored`).
    #[test]
    #[ignore]
    fn debit_cpu() {
        let dir = std::env::var("LCS_BENCH_DIR").expect("LCS_BENCH_DIR");
        let texts: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().take(256)
            .filter_map(|e| std::fs::read_to_string(e.path()).ok()).collect();
        let mut config = Config::default();
        config.device = "cpu".into();
        config.model.cpu_threads = std::env::var("LCS_THREADS").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
        config.model.batch_size = std::env::var("LCS_BATCH").ok().and_then(|t| t.parse().ok()).unwrap_or(32);
        if let Ok(f) = std::env::var("LCS_ONNX") { config.model.onnx_file = f; }
        if let Ok(m) = std::env::var("LCS_MODEL") { config.model.embedder = m; }
        let mut e = load(&config).unwrap();
        let t = std::time::Instant::now();
        e.embed_documents(&texts).unwrap();
        println!("DEBIT threads={} batch={} : {:.1} textes/s", config.model.cpu_threads, config.model.batch_size, texts.len() as f64 / t.elapsed().as_secs_f64());
    }

    /// Latence d'une question selon le backend (`LCS_DEVICE`, `LCS_ONNX`) (`--ignored`).
    #[test]
    #[ignore]
    fn latence_question() {
        let mut config = Config::default();
        config.device = std::env::var("LCS_DEVICE").unwrap_or_else(|_| "cpu".into());
        if let Ok(f) = std::env::var("LCS_ONNX") { config.model.onnx_file = f; }
        config.model.cpu_threads = std::env::var("LCS_THREADS").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
        let mut e = load(&config).unwrap();
        let questions = ["0-dimensional biomaterials show inductive properties in tissue engineering scaffolds", "where is the reciprocal rank fusion", "how are files hashed to detect changes", "load config file", "parse python decorators with tree sitter", "which function writes the vectors file atomically and compacts it"];
        let mut times = Vec::new();
        for _ in 0..4 { for q in questions { let t = std::time::Instant::now(); e.embed_query(q).unwrap(); times.push(t.elapsed().as_millis()); } }
        times.sort();
        println!("LATENCE {} : p50 {} ms, p95 {} ms", e.backend, times[times.len() / 2], times[times.len() * 95 / 100]);
    }
}
