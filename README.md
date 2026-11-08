<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/lcs-dark.svg">
    <img src="assets/lcs.svg" alt="lcs" width="220">
  </picture>
</p>

# local-code-search

Local code search for AI coding agents (Claude Code, Cursor…). It hands the agent the right piece
of code (`file:line` + snippet), found by meaning and by keywords, over MCP. Embedded ONNX model,
automatic GPU, no server, no cloud, no telemetry.

## Benchmark: first answer on real tickets

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/benchmark-dark.svg">
    <img src="assets/benchmark-light.svg" alt="Time to first answer 27% shorter with lcs: 31.5 s on average versus 43.4 s for Opus 5.5 alone, faster on 11 of 13 real tickets" width="760">
  </picture>
</p>

|  | Opus 5.5 alone | Opus 5.5 + lcs |
|---|---|---|
| Mean time to first answer | 43.4 s | **31.5 s (−27%)** |
| Faster on | 2 of 13 tickets | **11 of 13 tickets** |
| Mean cost per ticket | $0.56 | **$0.53 (−5%)** |
| Files to change found | 74% | **77%** |

13 real client tickets written by product owners, on a production codebase several years old
(about 4,000 files). Each ticket is replayed at the commit of its time in a fresh Claude Code
session, with the same configuration and tools for both setups (one run each); the answer is
compared with the files actually changed. The client code and the tickets stay private: only these
aggregates are published.

## Install

Download a binary from the [latest release](https://github.com/djdevpro/local-code-search/releases/latest)
(Windows x64, Linux x64 glibc ≥ 2.38, macOS 13.4+ Apple Silicon), or build it:

```bash
cargo install --git https://github.com/djdevpro/local-code-search
```

`lcs update` then keeps it up to date (SHA-256 verified).

## Quick start

```bash
lcs --root /path/to/project index      # keywords right away, then vectors
lcs --root /path/to/project search "where is the keyboard shortcut handled?"
lcs --root /path/to/project init       # hooks lcs into Claude Code / Cursor (MCP)
```

| Command | What it does |
|---|---|
| `index [--no-embed]` | updates the index (only changed files are re-read) |
| `search <query> [--mode bm25\|vector\|hybrid] [-n N] [--json] [--passages]` | ranked results, one per file (`--passages`: one per passage) |
| `def <name>` | exact definition of a symbol (`Parent::name` accepted) |
| `serve` | MCP server over stdio (`search`, `def`, `status`), keeps the index fresh |
| `watch` | re-indexes on every change |
| `init [--dry-run]` | writes `.mcp.json` (lcs tools always loaded, other servers untouched), `.cursor/mcp.json`, a CLAUDE.md section (with `.bak` backups) |
| `status` / `config` | index statistics / effective configuration |
| `bench <set.jsonl>` | evaluation: one `{"query", "expected": [paths]}` line per question |
| `update [--check]` | switches to the latest release |

Global options: `--root <folder>`, `--device auto|cpu|cuda|directml|coreml`.
Optional `local-code-search.toml` at the project root: model, reranker, device, languages,
exclusions, fusion weights, token budget.

## Performance

- Indexing: 50,000 files in 43 s (keywords); unchanged files skipped in milliseconds.
- Search: 23–40 ms p95 (hybrid), 8–10 ms (keywords only).
- Embeddings: 66–140 chunks/s on GPU (DirectML), cached on disk. 1.4 GB RAM for 50,000 files.

## How it works

```
index  : walk → hash → parse (tree-sitter) → chunk (1 symbol = 1 chunk) → BM25 (tantivy)
         → batched embeddings (fastembed/ort) → vectors.bin (f16, exact search)
search : (BM25 ∥ vectors) → weighted RRF fusion → optional reranker → one result per file
         (other matching passages listed under it) → compact answer
```

Queries work best in the language of the code (usually English) and with its words; lcs tells the
agent so in its MCP instructions and tool description, even when the user writes in another language.

Symbol-level chunking for Python, TypeScript/JavaScript, Rust, Go and PHP; other text files are
indexed as line windows. The only outbound connections are model downloads and `lcs update`, both
on demand; `tests/egress.rs` fails if any other appears.

License: MIT OR Apache-2.0.
