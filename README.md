<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/lcs-dark.svg">
    <img src="assets/lcs.svg" alt="lcs" width="220">
  </picture>
</p>

# local-code-search

Recherche de code locale pour agents IA (Claude Code, Cursor…) : le bon passage de code
(`fichier:ligne` + extrait), par le sens et par les mots-clés, via MCP. Modèle Hugging Face
embarqué (ONNX), GPU automatique, sans serveur, sans cloud, sans télémétrie.

## Installer

Prends le binaire de ton système dans la [dernière version](https://github.com/djdevpro/local-code-search/releases/latest)
(Windows x64, Linux x64 avec glibc ≥ 2.38, macOS 13.4+ sur Apple Silicon), ou compile-le. Ensuite,
`lcs update` le met à jour tout seul (somme SHA-256 vérifiée).

```bash
cargo install --git https://github.com/djdevpro/local-code-search
```

## Démarrage

```bash
cargo build --release
target/release/lcs --root /chemin/du/projet index      # BM25 immédiat, puis vecteurs
target/release/lcs --root /chemin/du/projet search "où est géré le raccourci clavier ?"
target/release/lcs --root /chemin/du/projet init       # branche Claude Code / Cursor (MCP)
```

| Commande | Rôle |
|---|---|
| `index [--no-embed]` | met l'index à jour (seuls les fichiers modifiés sont relus) |
| `search <question> [--mode bm25\|vector\|hybrid] [-n N] [--json]` | passages classés |
| `def <nom>` | définition exacte d'un symbole (`Parent::nom` accepté) |
| `serve` | serveur MCP stdio (outils `search`, `def`, `status`), index tenu à jour |
| `watch` | surveille le projet et réindexe à chaque modification |
| `init [--dry-run]` | `.mcp.json`, `.cursor/mcp.json`, section CLAUDE.md (sauvegardes `.bak`) |
| `status` | fichiers, morceaux, couverture des vecteurs, compteurs |
| `config` | configuration effective |
| `bench <jeu.jsonl>` | banc d'essai : une ligne `{"query", "expected": [chemins]}` par question |
| `update [--check]` | passe à la dernière version publiée |

Options globales : `--root <dossier>`, `--device auto|cpu|cuda|directml|coreml`.

## Configuration

`local-code-search.toml` à la racine du projet indexé (facultatif, toutes les options ont une
valeur par défaut, les options inconnues sont refusées) : modèle et variantes ONNX CPU/GPU,
reranker, backend, langages, exclusions, poids de la fusion, budget de tokens.

## Comment ça marche

```
index  : walk → hash → parse (tree-sitter) → chunk (1 symbole = 1 morceau) → BM25 (tantivy)
         → embeddings par lots (fastembed/ort, GPU auto) → vectors.bin (f16, recherche exacte)
search : (BM25 ∥ vecteurs) → fusion RRF pondérée déterministe → reranker (option) → réponse compacte
```

Connexions sortantes, toutes sur commande : le téléchargement d'un modèle (désactivable avec un
dossier local) et `lcs update`. Aucune télémétrie. Un test (`tests/egress.rs`) échoue si une autre
apparaît.

Langages découpés par symbole : Python, TypeScript / JavaScript, Rust, Go, PHP. Les autres fichiers
texte sont indexés par blocs de lignes.

Licence : MIT OR Apache-2.0.
