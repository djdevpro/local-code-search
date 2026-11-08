//! Composition des pipelines. Chaque étape vit dans son module ; ici on ne fait que les enchaîner.
//!
//! index  : walk → hash → parse → chunk → bm25 (immédiat) → embed → vectors
//! search : (bm25 ∥ vectors) → fuse → rerank → format

mod bm25;
mod chunk;
mod cli;
mod config;
mod def;
mod device;
mod embed;
mod eval;
mod format;
mod fuse;
mod hash;
mod init;
mod mcp;
mod parse;
mod rerank;
mod split;
mod status;
mod types;
mod update;
mod vectors;
mod walk;
mod watch;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use config::Config;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use types::{Chunk, FileEntry, Hit, Mode};

/// Version injectée par le workflow Release (dernier tag), sinon celle de Cargo.toml.
pub const VERSION: &str = match option_env!("LCS_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
const LEG_SIZE: usize = 50;
const EMBED_BATCH: usize = 64;
const FILE_BATCH: usize = 500;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = walk::canonical(&cli.root)?;
    let config = config::load(&root, cli.device)?;

    match cli.command {
        Command::Config => print!("{}", toml::to_string_pretty(&config)?),
        Command::Update { check } => update::run(VERSION, check)?,
        Command::Init { dry_run } => {
            for action in init::init(&root, &std::env::current_exe()?, dry_run)? {
                println!("{action}");
            }
        }
        Command::Index { no_embed } => {
            let mut engine = Engine::open(root, config)?;
            let mut writer = engine.index.writer()?;
            eprintln!("{}", engine.reindex(&mut writer)?);
            if !no_embed {
                eprintln!("{}", engine.embed_missing(None)?);
            }
        }
        Command::Search { query, mode, limit, json } => {
            let mut engine = Engine::open(root, config)?;
            let (hits, notes) = engine.search(&query, mode, limit)?;
            match json {
                true => println!("{}", serde_json::to_string_pretty(&hits)?),
                false => print!("{}", engine.render(&query, &hits, &notes)),
            }
        }
        Command::Def { name, json } => {
            let engine = Engine::open(root, config)?;
            let hits = def::find(&engine.index, &name, engine.config.search.results)?;
            match json {
                true => println!("{}", serde_json::to_string_pretty(&hits)?),
                false => print!("{}", engine.render(&name, &hits, &[])),
            }
        }
        Command::Status => print!("{}", Engine::open(root, config)?.status()),
        Command::Watch => {
            let engine = Arc::new(Mutex::new(Engine::open(root, config)?));
            keep_fresh(engine, true)?;
        }
        Command::Serve => serve(Engine::open(root, config)?)?,
        Command::Bench { dataset, mode, out } => {
            let mut engine = Engine::open(root, config)?;
            let cases = eval::load(&dataset)?;
            if mode != Mode::Bm25 {
                // Chauffe : modèle et session des questions chargés hors des mesures de latence.
                let _ = engine.vector_leg("warm-up");
            }
            let report = eval::run(&cases, |q| Ok(engine.search(q, mode, Some(10))?.0))?;
            let json = serde_json::to_string_pretty(&serde_json::json!({ "dataset": dataset, "mode": mode, "report": report }))?;
            if let Some(path) = out {
                std::fs::write(path, &json)?;
            }
            println!("{json}");
        }
    }
    Ok(())
}

/// Le moteur : l'index et, chargés à la demande, le modèle, les vecteurs et le reranker.
struct Engine {
    root: PathBuf,
    config: Config,
    data: PathBuf,
    index: bm25::Bm25,
    embedder: Option<embed::Embedder>,
    vectors: Option<vectors::Store>,
    model_error: Option<String>,
    model_notes: Vec<String>,
    reranker: Option<rerank::Reranker>,
    /// Vecteurs à invalider : morceaux de fichiers modifiés (même identifiant, contenu nouveau).
    dirty: Vec<u64>,
    /// L'index a changé depuis la dernière vérification complète des vecteurs.
    index_changed: bool,
}

impl Engine {
    fn open(root: PathBuf, config: Config) -> Result<Engine> {
        let data = root.join(&config.index.data_dir);
        let index = bm25::open(&data.join("index"))?;
        Ok(Engine {
            root,
            config,
            data,
            index,
            embedder: None,
            vectors: None,
            model_error: None,
            model_notes: Vec::new(),
            reranker: None,
            dirty: Vec::new(),
            index_changed: false,
        })
    }

    /// Met l'index BM25 à jour : seuls les fichiers modifiés sont relus.
    fn reindex(&mut self, writer: &mut tantivy::IndexWriter) -> Result<String> {
        let t = Instant::now();
        let (files, skipped) = walk::files(&self.root, &self.config)?;
        let plan = hash::plan(&self.index.manifest()?, files);
        let (changed, touched, deleted, unchanged) = (plan.changed.len(), plan.touched.len(), plan.deleted.len(), plan.unchanged);
        let errors = std::sync::atomic::AtomicUsize::new(0);
        let languages = &self.config.index.languages;
        let changed_paths: HashSet<String> = plan.changed.iter().map(|(e, _)| e.rel.clone()).collect();
        let mut dirty = Vec::new();
        let todo: Vec<(FileEntry, String)> = plan.changed.into_iter().chain(plan.touched).collect();
        let mut deleted_pending = plan.deleted;
        if todo.is_empty() && !deleted_pending.is_empty() {
            self.index.apply(writer, &deleted_pending, Vec::new())?;
        }
        // Lots bornés : mémoire constante, un commit par lot (un arrêt ne perd que le lot en cours).
        for batch in todo.chunks(FILE_BATCH) {
            let updates: Vec<bm25::FileUpdate> = batch
                .par_iter()
                .map(|(entry, hash)| {
                    let chunks = extract(entry, languages).unwrap_or_else(|e| {
                        errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        eprintln!("⚠ {} : {e:#} (fichier ignoré, l'indexation continue)", entry.rel);
                        Vec::new()
                    });
                    bm25::FileUpdate { entry: entry.clone(), hash: hash.clone(), chunks }
                })
                .collect();
            for u in updates.iter().filter(|u| changed_paths.contains(&u.entry.rel)) {
                dirty.extend(u.chunks.iter().map(|c| c.id));
            }
            self.index.apply(writer, &std::mem::take(&mut deleted_pending), updates)?;
        }
        // Un symbole modifié garde son identifiant : son ancien vecteur doit être recalculé.
        match self.vectors.as_mut() {
            Some(store) => store.delete(&dirty)?,
            None => self.dirty.extend(dirty),
        }
        self.index_changed |= changed + deleted > 0;
        let ms = t.elapsed().as_millis() as u64;
        let mut stats = status::load(&self.data);
        stats.last_index_ms = ms;
        stats.last_index_files = (changed + touched + unchanged) as u64;
        status::save(&self.data, &stats);
        Ok(format!(
            "Index BM25 à jour en {ms} ms : {changed} fichiers (ré)indexés, {touched} à date changée (contenu identique), {deleted} supprimés, {unchanged} inchangés. \
             Ignorés : {} binaires, {} minifiés, {} trop gros ; {} erreurs.",
            skipped.binary,
            skipped.minified,
            skipped.too_large,
            errors.into_inner()
        ))
    }

    /// Charge le modèle et ouvre les vecteurs une seule fois ; l'échec est mémorisé et annoncé.
    fn ensure_models(&mut self) -> Result<(), String> {
        if self.embedder.is_some() {
            return Ok(());
        }
        if let Some(e) = &self.model_error {
            return Err(e.clone());
        }
        let loaded = embed::load(&self.config).and_then(|embedder| {
            let (store, note) = vectors::open(&self.data.join("vectors.bin"), embedder.dim, &embedder.identity)?;
            Ok((embedder, store, note))
        });
        match loaded {
            Ok((embedder, store, note)) => {
                eprintln!("Modèle {} chargé sur {}.", self.config.model.embedder, embedder.backend);
                for note in &embedder.notes {
                    eprintln!("⚠ {note}");
                }
                self.model_notes = embedder.notes.iter().cloned().chain(note).collect();
                let mut store = store;
                if let Err(e) = store.delete(&std::mem::take(&mut self.dirty)) {
                    eprintln!("⚠ invalidation des vecteurs modifiés : {e:#}");
                }
                self.embedder = Some(embedder);
                self.vectors = Some(store);
                Ok(())
            }
            Err(e) => {
                let msg = format!("modèle {} indisponible : {e:#}", self.config.model.embedder);
                self.model_error = Some(msg.clone());
                Err(msg)
            }
        }
    }

    /// Calcule les vecteurs manquants et supprime ceux des morceaux disparus.
    /// Avec `engine`, verrouille par lot pour ne jamais bloquer les recherches longtemps.
    fn embed_missing(&mut self, max_batches: Option<usize>) -> Result<String> {
        if let Err(e) = self.ensure_models() {
            return Ok(format!("⚠ {e} : recherche par mots-clés seulement."));
        }
        let t = Instant::now();
        // Rien n'a changé et chaque morceau a son vecteur : inutile de relire tous les textes
        // (78 000 sur un gros dépôt), ce qui bloquait les recherches au démarrage.
        let chunks = self.index.counts().1 as usize;
        if !self.index_changed && self.vectors.as_ref().is_some_and(|s| s.len() == chunks) {
            let backend = self.embedder.as_ref().map_or("?", |e| e.backend);
            return Ok(format!("\rVecteurs : 0 calculés (0/s sur {backend}), 0 supprimés, 0 restants."));
        }
        let texts = self.index.chunk_texts()?;
        let (embedder, store) = (self.embedder.as_mut().expect("chargé"), self.vectors.as_mut().expect("ouvert"));
        let live: HashSet<u64> = texts.iter().map(|(id, _)| *id).collect();
        let have = store.ids();
        let stale: Vec<u64> = have.difference(&live).copied().collect();
        store.delete(&stale)?;
        let missing: Vec<(u64, String)> = texts.into_iter().filter(|(id, _)| !have.contains(id)).collect();
        let total = missing.len();
        let mut done = 0;
        for batch in missing.chunks(EMBED_BATCH).take(max_batches.unwrap_or(usize::MAX)) {
            let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
            let vecs = embedder.embed_documents(&texts)?;
            store.put(batch.iter().map(|(id, _)| *id).zip(vecs).collect())?;
            done += batch.len();
            if max_batches.is_none() {
                eprint!("\rVecteurs : {done}/{total}");
            }
        }
        store.compact_if_needed()?;
        if total == done {
            self.index_changed = false;
        }
        let rate = done as f64 / t.elapsed().as_secs_f64().max(1e-3);
        Ok(format!("\rVecteurs : {done} calculés ({rate:.0}/s sur {}), {} supprimés, {} restants.", embedder.backend, stale.len(), total - done))
    }

    fn vector_leg(&mut self, query: &str) -> Result<Vec<Hit>, String> {
        self.ensure_models()?;
        let (embedder, store) = (self.embedder.as_mut().expect("chargé"), self.vectors.as_ref().expect("ouvert"));
        let q = embedder.embed_query(query).map_err(|e| e.to_string())?;
        let top = store.search(&q, LEG_SIZE).map_err(|e| e.to_string())?;
        let scores: HashMap<u64, f32> = top.into_iter().collect();
        let ids: Vec<u64> = scores.keys().copied().collect();
        let mut hits = self.index.by_ids(&ids).map_err(|e| e.to_string())?;
        hits.iter_mut().for_each(|h| h.score = scores[&h.id]);
        Ok(fuse::penalize_tests(hits, self.config.search.test_penalty))
    }

    fn search(&mut self, query: &str, mode: Mode, limit: Option<usize>) -> Result<(Vec<Hit>, Vec<String>)> {
        let t = Instant::now();
        let s = self.config.search.clone();
        let mut notes = Vec::new();
        let keyword = match mode {
            Mode::Vector => Vec::new(),
            _ => fuse::penalize_tests(self.index.search(query, LEG_SIZE)?, s.test_penalty),
        };
        let semantic = match mode {
            Mode::Bm25 => Vec::new(),
            _ => self.vector_leg(query).unwrap_or_else(|e| {
                notes.push(format!("{e} : recherche par mots-clés seulement"));
                Vec::new()
            }),
        };
        if mode != Mode::Bm25 {
            notes.extend(self.model_notes.iter().cloned());
            if let Some(store) = &self.vectors {
                let chunks = self.index.counts().1 as usize;
                if store.len() < chunks {
                    notes.push(format!("vecteurs calculés pour {} / {chunks} morceaux (lancer `lcs index`)", store.len()));
                }
            }
        }
        let mut hits = match (mode, semantic.is_empty()) {
            (Mode::Bm25, _) | (Mode::Hybrid, true) => keyword,
            (Mode::Vector, _) => semantic,
            (Mode::Hybrid, false) => fuse::rrf(vec![(keyword, s.bm25_weight), (semantic, s.vector_weight)], s.rrf_k),
        };
        if self.config.reranker.enabled {
            if self.reranker.is_none() {
                match rerank::load(&self.config) {
                    Ok(r) => {
                        self.model_notes.extend(r.notes.iter().cloned());
                        self.reranker = Some(r);
                    }
                    Err(e) => notes.push(format!("reranker indisponible : {e:#}")),
                }
            }
            if let Some(r) = self.reranker.as_mut() {
                hits = fuse::penalize_tests(r.rerank(query, hits, self.config.reranker.candidates)?, s.test_penalty);
            }
        }
        hits.truncate(limit.unwrap_or(s.results).max(1));
        status::record_search(&self.data, t.elapsed().as_millis() as u64, !hits.is_empty());
        Ok((hits, notes))
    }

    fn render(&self, query: &str, hits: &[Hit], notes: &[String]) -> String {
        let count = |t: &str| self.embedder.as_ref().map_or_else(|| format::approx_tokens(t), |e| e.count_tokens(t));
        format::render(query, hits, notes, self.config.output.token_budget, &count)
    }

    fn status(&self) -> String {
        let (files, chunks) = self.index.counts();
        status::render(&status::Info {
            root: &self.root,
            files,
            chunks,
            vectors: self.vectors.as_ref().map(|s| s.len()).or_else(|| vectors::peek(&self.data.join("vectors.bin"))),
            model: &self.config.model.embedder,
            backend: self.embedder.as_ref().map(|e| e.backend),
            disk_bytes: status::dir_size(&self.data),
            stats: status::load(&self.data),
        })
    }
}

/// Lit un fichier et le découpe en morceaux ; une panique du parseur est isolée.
fn extract(entry: &FileEntry, languages: &[String]) -> Result<Vec<Chunk>> {
    let bytes = std::fs::read(&entry.abs)?;
    let source = String::from_utf8_lossy(&bytes);
    let lang = parse::language_of(&entry.rel, languages).unwrap_or("text");
    let units = std::panic::catch_unwind(|| parse::units(lang, &source))
        .map_err(|_| anyhow::anyhow!("le parseur {lang} a planté"))?;
    Ok(chunk::chunks(&entry.rel, lang, units))
}

/// Index initial puis mise à jour à chaque modification ; les vecteurs sont calculés par lots
/// courts pour que les recherches concurrentes ne soient jamais bloquées longtemps.
fn keep_fresh(engine: Arc<Mutex<Engine>>, verbose: bool) -> Result<()> {
    let (root, data_dir) = {
        let e = engine.lock().expect("verrou");
        (e.root.clone(), e.config.index.data_dir.clone())
    };
    let mut writer = match engine.lock().expect("verrou").index.writer() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("⚠ {e:#} : mode lecture seule.");
            return Ok(());
        }
    };
    let refresh = |writer: &mut tantivy::IndexWriter| {
        let report = engine.lock().expect("verrou").reindex(writer);
        match report {
            Ok(r) if verbose => eprintln!("{r}"),
            Err(e) => eprintln!("⚠ indexation : {e:#}"),
            _ => {}
        }
        loop {
            let step = engine.lock().expect("verrou").embed_missing(Some(1));
            match step {
                Ok(r) if r.ends_with(" 0 restants.") || r.starts_with('⚠') => {
                    if verbose {
                        eprintln!("{}", r.trim_start_matches('\r'));
                    }
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("⚠ vecteurs : {e:#}");
                    break;
                }
            }
        }
    };
    refresh(&mut writer);
    watch::watch(&root, &data_dir, || refresh(&mut writer))
}

/// Vrai si des vecteurs existent déjà : sinon le préchargement est inutile (l'indexation chargera
/// le modèle de toute façon) et on ne télécharge rien avant d'en avoir besoin.
fn engine_has_vectors(engine: &Arc<Mutex<Engine>>) -> bool {
    let e = engine.lock().expect("verrou");
    vectors::peek(&e.data.join("vectors.bin")).is_some_and(|n| n > 0)
}

fn serve(engine: Engine) -> Result<()> {
    let engine = Arc::new(Mutex::new(engine));
    let background = engine.clone();
    std::thread::spawn(move || {
        // Préchargement : modèle, session des questions et vecteurs décodés avant la première
        // question de l'agent (sinon la première recherche d'une session coûte ≈ 4 s de plus).
        if engine_has_vectors(&background) {
            let _ = background.lock().expect("verrou").vector_leg("warm-up");
        }
        if let Err(e) = keep_fresh(background, false) {
            eprintln!("⚠ surveillance arrêtée : {e:#}");
        }
    });
    let handler: mcp::Handler = Arc::new(move |call| {
        let mut e = engine.lock().expect("verrou");
        let result = match call {
            mcp::Call::Search { query, limit } => e
                .search(&query, Mode::Hybrid, (limit > 0).then_some(limit))
                .map(|(hits, notes)| e.render(&query, &hits, &notes)),
            mcp::Call::Def { name } => {
                let n = e.config.search.results;
                def::find(&e.index, &name, n).map(|hits| e.render(&name, &hits, &[]))
            }
            mcp::Call::Status => Ok(e.status()),
        };
        result.unwrap_or_else(|err| format!("erreur : {err:#}"))
    });
    mcp::serve(handler)
}
