//! Reranker optionnel : cross-encodeur ONNX local qui relit (question, morceau) ensemble.
//! Désactivé par défaut ; activé seulement si le banc d'essai prouve un gain.

use crate::config::Config;
use crate::{device, embed, fuse};
use crate::types::Hit;
use anyhow::Result;
use fastembed::{AutoDevicePolicy, RerankInitOptionsUserDefined, TextRerank, UserDefinedRerankingModel};

pub struct Reranker {
    model: TextRerank,
    pub notes: Vec<String>,
}

pub fn load(config: &Config) -> Result<Reranker> {
    let r = &config.reranker;
    let files = embed::fetch(&r.model)?;
    let (model, _backend, notes) = device::try_each(&config.device, |provider| {
        let onnx = embed::onnx_for(&r.model, provider.is_some(), &r.onnx_file, &r.onnx_file_gpu)?;
        let user = UserDefinedRerankingModel::new(onnx, files.tokenizer.clone());
        let options = RerankInitOptionsUserDefined::new()
            .with_max_length(config.model.max_tokens.min(2048))
            .with_auto_device(AutoDevicePolicy::PreferCPU)
            .with_execution_providers(provider.into_iter().collect());
        TextRerank::try_new_from_user_defined(user, options).map_err(|e| anyhow::anyhow!("{e}"))
    })?;
    Ok(Reranker { model, notes })
}

impl Reranker {
    /// Reclasse les `candidates` premiers résultats ; le reste garde son ordre derrière.
    pub fn rerank(&mut self, query: &str, mut hits: Vec<Hit>, candidates: usize) -> Result<Vec<Hit>> {
        let tail = hits.split_off(candidates.min(hits.len()));
        let docs: Vec<String> = hits.iter().map(|h| format!("{}\n{}\n{}", h.path, h.signature, h.body)).collect();
        let results = self.model.rerank(query.to_string(), docs, false, None).map_err(|e| anyhow::anyhow!("{e}"))?;
        for r in results {
            hits[r.index].score = r.score;
        }
        fuse::sort(&mut hits);
        hits.extend(tail);
        Ok(hits)
    }
}
