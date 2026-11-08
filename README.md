<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/lcs-dark.svg">
    <img src="assets/lcs.svg" alt="lcs" width="220">
  </picture>
</p>

# local-code-search

Local code search for AI coding agents (Claude Code, Codex, Cursor…). It hands the agent the right piece
of code (`file:line` + snippet), found by meaning and by keywords, over MCP. Embedded ONNX model,
automatic GPU, no server, no cloud, no telemetry.

## Benchmark: first answer on real tickets

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/benchmark-dark.svg">
    <img src="assets/benchmark-light.svg" alt="Time to first answer versus cost per ticket: Opus 5.5 alone 43.4 s and $0.56; with lcs 0.1, 44.4 s and $0.47; with lcs 0.2, 31.5 s and $0.53" width="760">
  </picture>
</p>

|  | Opus 5.5 alone | + lcs 0.1 | + lcs 0.2 |
|---|---|---|---|
| Mean time to first answer | 43.4 s | 44.4 s | **31.5 s** |
| Faster than Opus alone on | — | 7 of 13 tickets | **11 of 13 tickets** |
| Mean cost per ticket | $0.56 | **$0.47** | $0.53 |
| Files to change found | 74% | 74% | **77%** |

With lcs 0.2 (main branch), Opus 5.5 reaches its first answer 27% sooner than alone (31.5 s
instead of 43.4 s), finds a few more of the files to change and still costs 5% less per ticket.
lcs 0.1 matched Opus alone on speed and accuracy for 15% less. What changed in 0.2: one result per
file, the agent is told to write its queries in the language of the code, and lcs tools are always
loaded.

**How it was measured.** 13 real client tickets written by product owners, on a production codebase
several years old (about 4,000 files). Each ticket is replayed at the commit of its time in a fresh
Claude Code session, with the same configuration and tools for every setup (one run each); the
answer is compared with the files actually changed. Model loading is excluded from the lcs 0.1 times
(0.2 preloads it). The client code and the tickets stay private: only these aggregates are published.

## Install

Download a binary from the [latest release](https://github.com/djdevpro/local-code-search/releases/latest)
(Windows x64, Linux x64 glibc ≥ 2.38, macOS 13.4+ Apple Silicon), or build it:

```bash
cargo install --git https://github.com/djdevpro/local-code-search
```

`lcs update` then keeps it up to date (SHA-256 verified).

## Quick start

```bash
lcs --root /path/to/project init       # hooks lcs into Claude Code, Codex and Cursor
lcs --root /path/to/project index      # keywords right away, then vectors
lcs --root /path/to/project search "where is the keyboard shortcut handled?"   # try it
```

| Command | What it does |
|---|---|
| `index [--no-embed]` | updates the index (only changed files are re-read) |
| `search <query> [--mode bm25\|vector\|hybrid] [-n N] [--json] [--passages]` | ranked results, one per file (`--passages`: one per passage) |
| `def <name>` | exact definition of a symbol (`Parent::name` accepted) |
| `serve` | MCP server over stdio (`search`, `def`, `status`), keeps the index fresh |
| `watch` | re-indexes on every change |
| `init [--dry-run]` | Claude Code: `.mcp.json` (lcs tools always loaded) and a CLAUDE.md section. Codex, if present: `.codex/config.toml` and an AGENTS.md section. Cursor, if present: `.cursor/mcp.json` and an always-applied rule. Other settings are kept, with `.bak` backups |
| `hook prompt` | Experimental `UserPromptSubmit` hook for Claude Code and Codex, off by default (`[hook] enabled = true`, then `init`): gives the agent the files probably involved before its first turn |
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
