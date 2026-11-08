use serde::Serialize;
use std::path::PathBuf;

/// Un fichier trouvé par le parcours.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub abs: PathBuf,
    /// Chemin relatif à la racine, séparateurs `/`.
    pub rel: String,
    pub size: u64,
    pub mtime: u64,
}

/// État d'un fichier déjà indexé (manifeste).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub size: u64,
    pub mtime: u64,
    pub hash: String,
}

/// Un symbole extrait par tree-sitter (ou une fenêtre de lignes).
#[derive(Debug, Clone)]
pub struct Unit {
    pub kind: String,
    pub name: String,
    pub parent: Option<String>,
    /// Lignes 1-based, incluses.
    pub start: usize,
    pub end: usize,
    pub signature: String,
    pub doc: String,
    pub body: String,
}

/// Un morceau indexé.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: u64,
    pub path: String,
    pub lang: String,
    pub kind: String,
    pub name: String,
    pub parent: String,
    pub start: usize,
    pub end: usize,
    pub signature: String,
    pub body: String,
    /// Texte indexé : en-tête (chemin, parent, signature, doc) + corps.
    pub text: String,
}

/// Un résultat de recherche.
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub id: u64,
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub name: String,
    pub parent: String,
    pub signature: String,
    #[serde(skip)]
    pub body: String,
    pub score: f32,
    /// Autres passages du même fichier, dans l'ordre du classement (résultats regroupés par fichier).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<Passage>,
}

/// Un autre passage d'un fichier déjà présent dans les résultats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Passage {
    pub start: usize,
    pub end: usize,
    pub name: String,
}

/// Mode de recherche.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
pub enum Mode {
    Bm25,
    Vector,
    Hybrid,
}
