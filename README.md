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

## Does it help the agent?

We gave Claude Code (Opus 5.5) 13 real tickets from a client project and asked it to find the files
to change, once on its own and once with lcs. Two things count: how much the agent had to search, and
whether it found the right files.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/benchmark-dark.svg">
    <img src="assets/benchmark-light.svg" alt="Steps per ticket vs. files found, top left is better: Opus 5.5 alone 11.1 steps and 74% of files found; with lcs 0.1, 9.5 steps and 74%; lcs 0.2, 9.1 steps and 77%; lcs 0.3, 8.7 steps and 73%; lcs 0.4, 9.7 steps and 73%" width="760">
  </picture>
</p>

**How to read it:** each dot is one setup. Further left means fewer steps (the agent searched less);
higher means more of the right files found. **Top left is best.**

|  | Opus 5.5 alone | with lcs 0.1 | 0.2 | 0.3 | 0.4 |
|---|---|---|---|---|---|
| Steps per ticket (fewer is better) | 11.1 | 9.5 | 9.1 | **8.7** | 9.7 |
| Files to change found (more is better) | 74% | 74% | **77%** | 73% | 73% |
| Cost per ticket | $0.56 | **$0.47** | $0.53 | **$0.47** | $0.52 |

**What it shows**

- With lcs, the agent needs **13–22% fewer steps** and costs up to 15% less, for the same accuracy.
- lcs makes the agent faster, not more accurate: every setup finds about three files out of four.
- The lcs versions are within noise of each other (one or two files out of 34); the gap that matters
  is with lcs versus without.

**Method.** A step is one round of tool calls (search, grep, file read) before the agent answers.
Each ticket is replayed at the commit of its time in a fresh session, one run per setup, on a
production codebase of about 4,000 files; the answer is compared with the files actually changed.
Time is not compared, because it depends on API load when the run happens. The code and the tickets
stay private.

## Install

Download a binary from the [latest release](https://github.com/djdevpro/local-code-search/releases/latest)
(Windows x64, Linux x64 glibc ≥ 2.38, macOS 13.4+ Apple Silicon), or build it:

```bash
cargo install --git https://github.com/djdevpro/local-code-search
```

`lcs update` keeps it up to date (SHA-256 verified).

## Quick start

```bash
lcs --root /path/to/project init       # hooks lcs into Claude Code, Codex and Cursor
lcs --root /path/to/project index      # keywords right away, then vectors
lcs --root /path/to/project search "where is the keyboard shortcut handled?"
```

| Command | What it does |
|---|---|
| `init [--dry-run]` | MCP config and agent instructions for Claude Code, Codex and Cursor (existing settings kept, `.bak` backups) |
| `index [--no-embed]` | updates the index (only changed files are re-read) |
| `search <query> [--mode bm25\|vector\|hybrid] [-n N] [--json]` | ranked results, one per file |
| `def <name>` | exact definition of a symbol (`Parent::name` accepted) |
| `serve` / `watch` | MCP server over stdio / re-index on every change |
| `status` / `config` | index statistics / effective configuration |
| `bench <set.jsonl>` | evaluation: one `{"query", "expected": [paths]}` line per question |
| `update [--check]` | switches to the latest release |

Global options: `--root <folder>`, `--device auto|cpu|cuda|directml|coreml`. Optional
`local-code-search.toml` at the project root (model, device, languages, exclusions, fusion weights).

## Performance

- Indexing: 50,000 files in 43 s (keywords); unchanged files skipped in milliseconds.
- Search: 23–40 ms p95 (hybrid), 8–10 ms (keywords only).
- Embeddings: 66–140 chunks/s on GPU, cached by content: back on a branch or commit already seen,
  re-indexing takes seconds instead of minutes.

## How it works

```
index  : walk → hash → parse (tree-sitter) → chunk (1 symbol = 1 chunk) → BM25 (tantivy)
         → embeddings (fastembed/ort, cached by content) → vectors.bin (f16, exact search)
search : (BM25 ∥ vectors) → weighted RRF fusion → optional reranker → one result per file
```

Symbol-level chunks for Python, TypeScript/JavaScript, Rust, Go and PHP; other text files as line
windows. No outbound connection except model downloads and `lcs update` (`tests/egress.rs` enforces it).

License: MIT OR Apache-2.0.
