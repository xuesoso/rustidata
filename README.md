# rustidata

[![CI](https://github.com/xuesoso/rustidata/actions/workflows/ci.yml/badge.svg)](https://github.com/xuesoso/rustidata/actions/workflows/ci.yml)

A fast terminal tabular-data multitool — a Rust take on [VisiData](https://visidata.org),
built for huge files.

```
cargo build --release
target/release/rustidata data.csv          # or .tsv .jsonl .json .parquet .arrow .db .xlsx
target/release/rustidata data.csv.gz       # gz/bz2/xz/zst decompressed automatically
cat data.csv | target/release/rustidata    # stdin
target/release/rustidata .                 # directory browser
```

## Why

| | visidata | rustidata |
|---|---|---|
| open 216MB / 5M-row csv | ~45 s (loads all rows into RAM) | **0.7 ms** to first screen; 71 ms full background index |
| memory for that file | gigabytes (Python objects) | ~40 MB index + reclaimable mmap pages |
| sort 5M rows | minutes | **1.4 s** (rayon-parallel, permutation only) |
| frequency table, 5M rows | minutes | **1.2 s** |
| delimiter detection | none (fixed option) | automatic (`,` `\t` `;` `\|`, quote-aware) |
| color themes | per-option config only | 7 built-in themes, live switcher, persisted |

The trick: plain text files are memory-mapped and only a row-offset index is
built (in a background thread, quote-aware when needed); rows are parsed on
demand with a sharded LRU cache. Compressed files and stdin stream into a
compact columnar store while the UI stays live. Sorting produces a RowId
permutation — data never moves.

## Keys (visidata-compatible)

Movement: `hjkl`/arrows, `g`-prefixed for far edges, `PgUp`/`PgDn`, `zr` go-to-row.

| key | action |
|---|---|
| `F` / `gF` | frequency table (current column / key columns) |
| `W` | pivot (group by key cols, spread current col) |
| `M` / `zM` / `T` | melt / unfurl / transpose |
| `I` | describe (summary statistics) |
| `&` | join top two sheets (7 join types) |
| `[` `]` / `z[` `z]` | sort (replace / additive) |
| `/` `?` `n` `N`, `g/` | regex search |
| `s` `t` `u`, `gs` `gt` `gu`, `\|` `\` `z\|` | selection (row / all / regex / expression) |
| `e` `ge` `a` `d` `gd` | edit cell, set selected, add row, delete |
| `y` `gy` `zy` `p` `zp` | clipboard (rows / cells, system clipboard synced) |
| `!` `-` `^` `H` `L` | key-col, hide, rename, slide column |
| `_` `<` `>` `z_` | width: toggle fit-to-content, narrower / wider by 1, set exact |
| `~` `#` `%` `@` `$` | column type: str / int / float / date / currency |
| `=` `:` `;` `*` | expression column, regex split / capture / substitute |
| `'` / `g'` | freeze column / sheet |
| `f` | fill nulls down |
| `.` | braille scatter plot (`hjkl` pan, `+`/`-` zoom, `,`/`.` dot size, `0` reset) |
| `U` / `R` | undo / redo |
| `S` `C` `O` | sheets / columns / options metasheets |
| `Enter` | dive: row → detail (column/value), freq group → source rows, dir entry → file, db → table |
| `z Enter` | view current cell's full value in a scrollable, word-wrapped pager (`e` edit, `q` close) |
| `^S` | save (csv/tsv/json/jsonl/md/sqlite by extension) |
| `Space` | run any command by name |
| `F8` / `F9` | choose theme (live preview) / cycle theme |
| `F10` | menu bar |
| `z?` / `F1` | help |
| `q` / `gq` | close sheet / quit |

Expressions use [Rhai](https://rhai.rs) with column names as variables:
`=profit=revenue - cost`, select-by-expression `z| score > 90`.
Helpers: `str()`, `int()`, `float()`, `len()`.

## Config & themes

Config lives at `~/.config/rustidata/config` (TOML):

```toml
[options]
delimiter = ""        # empty = sniff automatically
default_width = 20
col_gap = 3           # cells between columns (separator glyph centered in it)
plot_dot_radius = 1   # scatter-plot point radius in subpixels (0 = single dot)

[theme]
name = "gruvbox"      # default-dark | light | monochrome | ascii | solarized-dark | gruvbox | gruvbox-light

[colors]              # per-slot overrides on top of the theme
current_row = "bold white on 24"

[keybindings]
"zq" = "quit-sheet"
```

The theme switcher (`F8`/`F9`) persists your choice here automatically,
preserving comments.

## Workspace

- `crates/core` — data engine (stores, transforms, expressions); no terminal deps
- `crates/loaders` — format loaders (csv/tsv/json/fixed + feature-gated parquet/arrow/sqlite/xlsx)
- `crates/tui` — the binary

```
cargo test --workspace            # 138 tests
scripts/bench_vs_visidata.sh     # generate data, benchmark both tools
```

Not yet ported from visidata: macros, command-log replay, plugins,
multi-pane splits, mouse support.

## Security posture

- **No network access, no telemetry.** The binary contains no networking code
  and no dependency capable of it (verified by dependency-tree scan and binary
  symbol inspection). The only environment variable read is
  `XDG_CONFIG_HOME`; no subprocesses are ever spawned.
- **Writes** happen only on explicit save (`^S`) and theme persistence to
  `~/.config/rustidata/config`. Yanking also copies to the system clipboard
  (disable by building with `--no-default-features`, dropping the
  `system-clipboard` feature).
- **One `unsafe` block** in the codebase (the mmap in
  `core/src/store/mmap_text.rs`). Caveat shared by all mmap-based tools: if
  another process truncates a file while it is open, the OS terminates the
  viewer (SIGBUS); this is a crash, not memory corruption.
- **Untrusted files**: SQL identifiers from database files are quoted, screen
  output is control-character-sanitized (no terminal escape injection),
  expression evaluation is sandboxed (Rhai, 1M-operation cap, no IO),
  file-derived column counts are capped at 10,000. Known limit: decompressed
  streams (e.g. `.csv.gz`) materialize in RAM, so a decompression bomb can
  exhaust memory — same exposure as visidata.
- `cargo audit`: zero known vulnerabilities (one informational advisory for
  `paste`, an unmaintained build-time proc-macro pulled by `parquet`).
- **Platforms**: macOS and Linux are first-class; the workspace builds and the
  full test suite passes on Debian bookworm, and the TUI is smoke-tested in a
  Linux pty.
