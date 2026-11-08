<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/lcs-dark.svg">
    <img src="assets/lcs.svg" alt="lcs" width="220">
  </picture>
</p>

# local-code-search

Local code search for AI coding agents (Claude Code, Cursor…): it hands the agent the right piece
of code (`file:line` + snippet), found by meaning and by keywords, over MCP. Embedded Hugging Face
model (ONNX), automatic GPU, no server, no cloud, no telemetry.

## Benchmark: first answer on real tickets

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/benchmark-dark.svg">
    <img src="assets/benchmark-light.svg" alt="Time to first answer per request, Opus 5.5 alone versus Opus 5.5 with lcs" width="760">
  </picture>
</p>

|  | Opus 5.5 alone | Opus 5.5 + lcs |
|---|---|---|
| Mean time to first answer | 43.4 s | 44.4 s |
| Fastest on | 6 of 13 tickets | **7 of 13 tickets** |
| Mean cost per ticket | $0.56 | **$0.47 (−15%)** |
| Files to change found | 74% | 74% |

**How it was measured.** 13 real client tickets, written by product owners, on a legacy
production codebase that is several years old (about 4,000 files). Each ticket is replayed
exactly as it was asked: the repository is reset to the commit in effect at that time, and a
fresh Claude Code session (Opus 5.5, no prior context) must find the files to change. "Files to
change found" compares its answer with the files actually changed when the ticket was handled.
Both setups have the same read-only tools; the second one can also query lcs.

The lcs figures exclude the model loading at the start of each session, which now happens in the
background while the agent thinks. The client code and the tickets stay private: only these
aggregate results are published.

**Takeaway:** same accuracy, same speed, 15% cheaper. The agent reads less to reach the same answer.

## Install

Download the binary for your system from the [latest release](https://github.com/djdevpro/local-code-search/releases/latest)
(Windows x64, Linux x64 with glibc ≥ 2.38, macOS 13.4+ on Apple Silicon), or build it. Then
`lcs update` keeps it up to date (SHA-256 checksum verified).

```bash
cargo install --git https://github.com/djdevpro/local-code-search
```

## Quick start

```bash
lcs --root /path/to/project index      # keywords right away, then vectors
lcs --root /path/to/project search "where is the keyboard shortcut handled?"
lcs --root /path/to/project init       # hooks lcs into Claude Code / Cursor (MCP)
```

| Command | What it does |
|---|---|
| `index [--no-embed]` | updates the index (only changed files are read again) |
| `search <query> [--mode bm25\|vector\|hybrid] [-n N] [--json]` | ranked passages |
| `def <name>` | exact definition of a symbol (`Parent::name` accepted) |
| `serve` | MCP server over stdio (`search`, `def`, `status` tools), keeps the index fresh |
| `watch` | watches the project and re-indexes on every change |
| `init [--dry-run]` | `.mcp.json`, `.cursor/mcp.json`, CLAUDE.md section (`.bak` backups) |
| `status` | files, chunks, vector coverage, counters |
| `config` | effective configuration |
| `bench <set.jsonl>` | evaluation: one `{"query", "expected": [paths]}` line per question |
| `update [--check]` | switches to the latest published release |

Global options: `--root <folder>`, `--device auto|cpu|cuda|directml|coreml`.

## Performance

- Indexing: 50,000 files in 43 s (keywords); unchanged files are skipped in milliseconds.
- Search: 23–40 ms at p95 (hybrid), 8–10 ms (keywords only).
- Memory: 1.4 GB while indexing 50,000 files with vectors.
- Embeddings: 66–140 chunks/s on GPU (DirectML), all computed once and cached on disk.

## Configuration

`local-code-search.toml` at the root of the indexed project (optional; every option has a
default, unknown options are rejected): model and CPU/GPU ONNX variants, reranker, device,
languages, exclusions, fusion weights, token budget.

## How it works

```
index  : walk → hash → parse (tree-sitter) → chunk (1 symbol = 1 chunk) → BM25 (tantivy)
         → batched embeddings (fastembed/ort, automatic GPU) → vectors.bin (f16, exact search)
search : (BM25 ∥ vectors) → weighted, deterministic RRF fusion → reranker (optional) → compact answer
```

When the MCP server starts, the model and the vectors are preloaded in the background, so the
agent's first search does not wait.

Outbound connections, all on demand: downloading a model (can be replaced by a local folder) and
`lcs update`. No telemetry. A test (`tests/egress.rs`) fails if any other one appears.

Languages split by symbol: Python, TypeScript / JavaScript, Rust, Go, PHP. Other text files are
indexed as line windows.

License: MIT OR Apache-2.0.
