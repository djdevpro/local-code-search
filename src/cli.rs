use crate::types::Mode;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "lcs", version = crate::VERSION, about = "Recherche de code locale pour agents IA")]
pub struct Cli {
    /// Racine du projet à indexer (contient éventuellement local-code-search.toml)
    #[arg(long, global = true, default_value = ".")]
    pub root: PathBuf,

    /// Backend matériel : auto | cpu | cuda | directml | coreml (prime sur la config)
    #[arg(long, global = true)]
    pub device: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Indexe (ou met à jour) le projet
    Index {
        /// BM25 seulement, sans calculer les vecteurs
        #[arg(long)]
        no_embed: bool,
    },
    /// Cherche des passages de code
    Search {
        query: String,
        #[arg(long, value_enum, default_value = "hybrid")]
        mode: Mode,
        /// Nombre de résultats (défaut : configuration)
        #[arg(long, short = 'n')]
        limit: Option<usize>,
        /// Sortie JSON (pour les scripts)
        #[arg(long)]
        json: bool,
        /// Un résultat par passage, sans regrouper par fichier
        #[arg(long)]
        passages: bool,
    },
    /// Trouve la définition exacte d'un symbole
    Def {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Lance le serveur MCP (stdio), avec mise à jour automatique de l'index
    Serve,
    /// Surveille le projet et met l'index à jour à chaque modification
    Watch,
    /// Branche l'outil dans Claude Code, Codex et Cursor (MCP et consignes)
    Init {
        #[arg(long)]
        dry_run: bool,
    },
    /// État de l'index et compteurs
    Status,
    /// Affiche la configuration effective
    Config,
    /// Passe à la dernière version publiée (somme SHA-256 vérifiée)
    Update {
        /// Vérifie seulement s'il existe une nouvelle version
        #[arg(long)]
        check: bool,
    },
    /// Banc d'essai sur un jeu JSONL {"query", "expected": [chemins]}
    Bench {
        dataset: PathBuf,
        #[arg(long, value_enum, default_value = "hybrid")]
        mode: Mode,
        /// Écrit le rapport JSON dans ce fichier
        #[arg(long)]
        out: Option<PathBuf>,
    },
}
