//! Index tantivy unique : morceaux (recherche BM25, analyseur code), noms de symboles et
//! manifeste des fichiers (taille, date, hash). Un seul IDF global.

use crate::split::{self, CodeTokenizer};
use crate::types::{Chunk, FileEntry, FileState, Hit};
use anyhow::{Context, Result};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use tantivy::collector::{DocSetCollector, TopDocs};
use tantivy::query::{BooleanQuery, BoostQuery, ConstScoreQuery, Occur, PhraseQuery, Query, TermQuery};
use tantivy::schema::{
    Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::TextAnalyzer;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

const WRITER_HEAP: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Fields {
    doctype: Field,
    id: Field,
    path: Field,
    lang: Field,
    kind: Field,
    name: Field,
    name_exact: Field,
    parent: Field,
    start: Field,
    end: Field,
    signature: Field,
    body: Field,
    text: Field,
    size: Field,
    mtime: Field,
    hash: Field,
}

pub struct Bm25 {
    index: Index,
    reader: IndexReader,
    f: Fields,
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let code = TextFieldIndexing::default()
        .set_tokenizer(split::NAME)
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let code_stored = TextOptions::default().set_indexing_options(code.clone()).set_stored();
    let f = Fields {
        doctype: b.add_text_field("doctype", STRING),
        id: b.add_u64_field("id", INDEXED | STORED),
        path: b.add_text_field("path", STRING | STORED),
        lang: b.add_text_field("lang", STRING | STORED),
        kind: b.add_text_field("kind", STRING | STORED),
        name: b.add_text_field("name", TextOptions::default().set_indexing_options(code)),
        name_exact: b.add_text_field("name_exact", STRING | STORED),
        parent: b.add_text_field("parent", STORED),
        start: b.add_u64_field("start", STORED),
        end: b.add_u64_field("end", STORED),
        signature: b.add_text_field("signature", STORED),
        body: b.add_text_field("body", STORED),
        text: b.add_text_field("text", code_stored),
        size: b.add_u64_field("size", STORED),
        mtime: b.add_u64_field("mtime", STORED),
        hash: b.add_text_field("hash", STORED),
    };
    (b.build(), f)
}

pub fn open(dir: &Path) -> Result<Bm25> {
    std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    let (schema, f) = schema();
    let directory = tantivy::directory::MmapDirectory::open(dir)?;
    let index = Index::open_or_create(directory, schema).context("ouverture de l'index tantivy")?;
    index.tokenizers().register(split::NAME, TextAnalyzer::builder(CodeTokenizer::default()).build());
    let reader = index.reader_builder().reload_policy(ReloadPolicy::OnCommitWithDelay).try_into()?;
    Ok(Bm25 { index, reader, f })
}

/// Un fichier à (ré)indexer : son état et ses morceaux.
pub struct FileUpdate {
    pub entry: FileEntry,
    pub hash: String,
    pub chunks: Vec<Chunk>,
}

impl Bm25 {
    /// Un seul processus écrit ; les autres lisent.
    pub fn writer(&self) -> Result<IndexWriter> {
        self.index
            .writer(WRITER_HEAP)
            .context("index verrouillé : un autre processus lcs est déjà en train d'indexer")
    }

    /// Applique un lot de changements puis valide (commit atomique). Sous Windows, un fichier
    /// tout juste créé peut être verrouillé un instant (antivirus, fusion de segments) : en cas
    /// d'échec, retour au dernier commit et nouvel essai, jusqu'à 3 fois.
    pub fn apply(&self, writer: &mut IndexWriter, deleted: &[String], updates: Vec<FileUpdate>) -> Result<()> {
        let mut attempt = 0;
        loop {
            match self.try_apply(writer, deleted, &updates) {
                Ok(()) => return Ok(()),
                Err(e) if attempt < 3 => {
                    attempt += 1;
                    eprintln!("⚠ écriture de l'index refusée ({e:#}) : nouvel essai {attempt}/3");
                    writer.rollback()?;
                    std::thread::sleep(std::time::Duration::from_millis(300 * attempt));
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn try_apply(&self, writer: &mut IndexWriter, deleted: &[String], updates: &[FileUpdate]) -> Result<()> {
        let f = self.f;
        for path in deleted.iter().map(String::as_str).chain(updates.iter().map(|u| u.entry.rel.as_str())) {
            writer.delete_term(Term::from_field_text(f.path, path));
        }
        for u in updates {
            writer.add_document(doc!(
                f.doctype => "file", f.path => u.entry.rel.clone(), f.size => u.entry.size,
                f.mtime => u.entry.mtime, f.hash => u.hash.clone(),
            ))?;
            for c in &u.chunks {
                writer.add_document(doc!(
                    f.doctype => "chunk", f.id => c.id, f.path => c.path.clone(), f.lang => c.lang.clone(),
                    f.kind => c.kind.clone(), f.name => c.name.clone(), f.name_exact => c.name.clone(),
                    f.parent => c.parent.clone(), f.start => c.start as u64, f.end => c.end as u64,
                    f.signature => c.signature.clone(), f.body => c.body.clone(), f.text => c.text.clone(),
                ))?;
            }
        }
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    fn all(&self, doctype: &str) -> Result<Vec<TantivyDocument>> {
        let searcher = self.reader.searcher();
        let q = TermQuery::new(Term::from_field_text(self.f.doctype, doctype), IndexRecordOption::Basic);
        let mut addrs: Vec<_> = searcher.search(&q, &DocSetCollector)?.into_iter().collect();
        addrs.sort();
        addrs.into_iter().map(|a| Ok(searcher.doc(a)?)).collect()
    }

    pub fn manifest(&self) -> Result<HashMap<String, FileState>> {
        let f = self.f;
        Ok(self
            .all("file")?
            .iter()
            .map(|d| {
                let state = FileState { size: u64_of(d, f.size), mtime: u64_of(d, f.mtime), hash: str_of(d, f.hash) };
                (str_of(d, f.path), state)
            })
            .collect())
    }

    /// (id, texte) de tous les morceaux : source des embeddings.
    pub fn chunk_texts(&self) -> Result<Vec<(u64, String)>> {
        Ok(self.all("chunk")?.iter().map(|d| (u64_of(d, self.f.id), str_of(d, self.f.text))).collect())
    }

    pub fn counts(&self) -> (u64, u64) {
        let searcher = self.reader.searcher();
        let count = |t: &str| {
            let q = TermQuery::new(Term::from_field_text(self.f.doctype, t), IndexRecordOption::Basic);
            searcher.search(&q, &tantivy::collector::Count).unwrap_or(0) as u64
        };
        (count("file"), count("chunk"))
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>> {
        let f = self.f;
        let tokens = split::tokens(query);
        let mut terms = BTreeSet::new();
        for (whole, subs) in &tokens {
            terms.insert(whole.clone());
            terms.extend(subs.iter().cloned());
        }
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        for t in &terms {
            clauses.push((Occur::Should, term(f.text, t)));
            clauses.push((Occur::Should, Box::new(BoostQuery::new(term(f.name, t), 2.0))));
        }
        let raw: Vec<&str> = query.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
        if raw.len() == 1 {
            clauses.push((Occur::Should, Box::new(BoostQuery::new(term(f.name_exact, raw[0]), 8.0))));
        }
        if tokens.len() >= 2 {
            let phrase = tokens.iter().map(|(w, _)| Term::from_field_text(f.text, w)).collect();
            clauses.push((Occur::Should, Box::new(BoostQuery::new(Box::new(PhraseQuery::new(phrase)), 3.0))));
        }
        if clauses.is_empty() {
            return Ok(Vec::new());
        }
        self.run(Box::new(BooleanQuery::new(clauses)), limit)
    }

    /// Morceaux dont le nom est exactement `name` (sensible à la casse), sinon insensible.
    pub fn by_name(&self, name: &str, limit: usize) -> Result<Vec<Hit>> {
        let exact = self.run(term(self.f.name_exact, name), limit)?;
        if !exact.is_empty() {
            return Ok(exact);
        }
        self.run(term(self.f.name, &name.to_lowercase()), limit)
    }

    pub fn by_ids(&self, ids: &[u64]) -> Result<Vec<Hit>> {
        let clauses = ids
            .iter()
            .map(|&id| -> (Occur, Box<dyn Query>) {
                (Occur::Should, Box::new(TermQuery::new(Term::from_field_u64(self.f.id, id), IndexRecordOption::Basic)))
            })
            .collect::<Vec<_>>();
        if clauses.is_empty() {
            return Ok(Vec::new());
        }
        self.run(Box::new(BooleanQuery::new(clauses)), ids.len())
    }

    fn run(&self, query: Box<dyn Query>, limit: usize) -> Result<Vec<Hit>> {
        let f = self.f;
        let only_chunks = ConstScoreQuery::new(term(f.doctype, "chunk"), 0.0);
        let q = BooleanQuery::new(vec![(Occur::Must, query), (Occur::Must, Box::new(only_chunks))]);
        let searcher = self.reader.searcher();
        let top = searcher.search(&q, &TopDocs::with_limit(limit.max(1)).order_by_score())?;
        top.into_iter()
            .map(|(score, addr)| {
                let d: TantivyDocument = searcher.doc(addr)?;
                Ok(Hit {
                    id: u64_of(&d, f.id),
                    path: str_of(&d, f.path),
                    start: u64_of(&d, f.start) as usize,
                    end: u64_of(&d, f.end) as usize,
                    kind: str_of(&d, f.kind),
                    name: str_of(&d, f.name_exact),
                    parent: str_of(&d, f.parent),
                    signature: str_of(&d, f.signature),
                    body: str_of(&d, f.body),
                    score,
                })
            })
            .collect()
    }
}

fn term(field: Field, text: &str) -> Box<dyn Query> {
    Box::new(TermQuery::new(Term::from_field_text(field, text), IndexRecordOption::WithFreqs))
}

fn str_of(d: &TantivyDocument, f: Field) -> String {
    d.get_first(f).and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn u64_of(d: &TantivyDocument, f: Field) -> u64 {
    d.get_first(f).and_then(|v| v.as_u64()).unwrap_or(0)
}
