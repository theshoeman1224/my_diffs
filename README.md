# mydiff

Compare two files or folders and see exactly what changed: text, directories,
binary files and structured data (JSON, YAML, CSV). Use it as an interactive
terminal viewer or as a scriptable CLI.

- **Red** = removed · **green** = added · **yellow** = modified (changed words highlighted)
- Quick toggles for whitespace, case, blank lines and line endings
- Ignore/whitelist regexes, masking (for example timestamps), include/exclude globs
- Config file with named profiles
- Runs anywhere a terminal does: xterm, or Windows Terminal/PuTTY/MobaXterm over SSH

## Build

```sh
cargo build --release          # binary at target/release/mydiff
```

## Simple mode: the viewer

```sh
mydiff old.txt new.txt         # opens the viewer when stdout is a terminal
mydiff dir_a/ dir_b/           # folder list; Enter opens a file, Esc returns
```

| Key | Action |
|---|---|
| `n` / `p` | next / previous change |
| `j` `k` ↑ ↓ wheel, PgUp/PgDn, `g`/`G` | scroll |
| `h` `l` ← → | scroll horizontally |
| `w` | whitespace mode: exact → trailing → change → all |
| `i` / `b` / `e` | ignore case / blank lines / line endings |
| `u` | side-by-side ↔ inline layout |
| `s` | folder view: show/hide identical files |
| `?` | help · `q`/`Esc` quit or back · `Ctrl-c` quit |

Viewing from Windows: run `mydiff` on the Linux host inside your SSH session
(Windows Terminal, PuTTY or MobaXterm), or in an `xterm` shown through an X server.
The viewer uses only the 16 standard terminal colors, and every change also has a
marker (`-` `+` `~`), so it stays readable in limited terminals.

## Advanced mode: the CLI

```sh
mydiff -f unified a.txt b.txt                 # patch-style (default when piped)
mydiff -f side-by-side --width 160 a b        # two columns
mydiff -f summary dir_a dir_b                 # one line per difference + totals
mydiff -f json a.json b.json                  # machine-readable
mydiff -q a b && echo same                    # exit status only
```

Exit status: `0` identical, `1` different, `2` error.

Common options (see `mydiff --help` for all):

| Option | Meaning |
|---|---|
| `-w` / `-b` / `-Z` / `--whitespace MODE` | ignore all / changed amount of / trailing whitespace |
| `-i`, `-B`, `--strict-line-endings` | ignore case, ignore blank lines, treat CRLF ≠ LF |
| `--ignore REGEX` | don't compare matching lines (repeatable) |
| `--only REGEX` | compare **only** matching lines (whitelist, repeatable) |
| `--mask 'REGEX=>REPL'` | replace before comparing, e.g. `--mask '\d{4}-\d\d-\d\d=>DATE'` |
| `--include GLOB` / `--exclude GLOB` | folder filters (`*.rs` matches at any depth) |
| `--key-column NAME` | CSV: match rows by key |
| `--ignore-path '$.meta.time'` | JSON/YAML: skip a path (`[*]` and `.*` wildcards) |
| `--mode text\|dir\|binary\|json\|yaml\|csv` | override auto-detection |
| `-P NAME`, `--config FILE`, `--no-config` | profiles and config files |

## Config file

`mydiff config init` writes a commented `.mydiff.toml`. Files load in this order,
with later ones winning: `~/.config/mydiff/config.toml`, then `./.mydiff.toml`,
then `--config FILE`, then CLI flags. Later layers replace scalar settings and
append to lists.

```toml
[defaults]
whitespace = "trailing"
exclude = [".git", "target"]

[profile.logs]                 # mydiff -P logs a.log b.log
only = [' (ERROR|WARN) ']
[[profile.logs.mask]]
pattern = '^\S+ \S+'
replace = "<TIME>"
```

`mydiff config show` prints the effective options. `mydiff config paths` shows
which files were loaded.

## Development

The repo is test-first; see [CLAUDE.md](CLAUDE.md) for where each kind of test goes.

```sh
just setup                     # one-time: nextest, insta, llvm-cov, deny
just ci                        # what CI runs: fmt, clippy, tests, snapshots, coverage, deny
just new-case text/my_case     # scaffold a golden fixture, then `just snap`
```
