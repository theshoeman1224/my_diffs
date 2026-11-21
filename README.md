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
| `]c` / `[c` (also `p`) | next / previous change, as in vimdiff |
| `/` / `?` | search forward / backward in the active pane (see [Searching](#searching)) |
| `n` / `N` | next / previous match; in the folder and hex views, next / previous change |
| `Tab` or `Ctrl-w w` / `Ctrl-w h` / `Ctrl-w l` | make the other / left / right pane active |
| `j` `k` ↑ ↓ wheel, PgUp/PgDn, `g`/`G` | scroll |
| `h` `l` ← → | scroll horizontally (stops when the longest line's end is in view) |
| `w` | whitespace mode: exact → trailing → change → all |
| `i` / `b` / `e` | ignore case / blank lines / line endings |
| `u` | side-by-side ↔ inline layout |
| `s` | folder view: show/hide identical files |
| `F1` | show / hide the command sidebar (remembered between runs) |
| `:` | command line: `:help`, `:sidebar`, `:noh`, `:q`, `:qa` (see [Commands](#commands)) |
| `q` / `Esc` | quit or go back (Esc closes `:help` first) · `Ctrl-c` quit from anywhere |

#### The sidebar and `:help`

A sidebar listing every command for the current screen, with the current value
of each toggle, is open on the right by default. `F1` (or `:sidebar`) hides and
shows it, and `:sidebar left` / `:sidebar right` docks it on that edge (opening
it). The full key reference, with notes and the color legend, is `:help`.

The viewer **remembers the sidebar between runs**: whether it is open and which
edge it is on. Close it once you know the keys and it stays closed, including
for every file `git difftool` opens. The state is kept in
`$XDG_STATE_HOME/mydiff/state.toml` (usually `~/.local/state/mydiff/state.toml`),
not in your config file:

```toml
[sidebar]
open = false
dock = "left"
```

The file is only written when the setting changes. If it is missing or
unreadable the defaults apply (open, on the right), and if it cannot be saved
(for example a read-only home) the viewer carries on without it. Delete the
file to go back to the defaults.

The sidebar needs room next to the diff (94 columns for text, 84 for folders,
113 for hex), so it hides itself in a narrower terminal and comes back when the
window is widened; that does not change the remembered setting. A folder view
and the files opened from it share the same sidebar.

### Searching

Search works like vim's, in the text view (a file diff, including one
opened from a folder). The folder and hex views have no search.

- `/pattern` then Enter searches forward; `?pattern` searches backward.
  `n` repeats the search in the same direction, `N` in the opposite one.
  Searches wrap around the end of the file and say so in the status bar
  (`search hit BOTTOM, continuing at TOP`). The status bar shows the match
  number and the total, for example `/foo  [3/12]`.
- **Searches only look at the active pane.** The active pane's file name is
  marked `▶` in the header, and the left pane is active at start. **`Tab`**
  swaps to the other pane. vimdiff's window keys work too: `Ctrl-w w` (also
  `Ctrl-w Ctrl-w`, `Ctrl-w W`, `Ctrl-w p`) goes to the other pane, `Ctrl-w h` /
  `Ctrl-w ←` to the left one and `Ctrl-w l` / `Ctrl-w →` to the right one. In vim
  every window command is `Ctrl-w` followed by a key; `Tab` is this viewer's
  one-key shortcut. Both panes always scroll
  together, so a jump moves both. Switching panes keeps the pattern; press `n`
  to search the new pane. If a pattern is only in the other pane, the
  "not found" message says so: `E486: Pattern not found: foo (in right pane: Tab)`.
- In the inline layout (`u`), the active pane decides which lines can match:
  unchanged lines plus `-` lines for the left pane, or plus `+` lines for the
  right pane. Unchanged lines show the active pane's text. This matters when
  whitespace or case is ignored and the two sides differ only in that way.
- While you type, the view jumps to the first match and highlights every
  match (vim's `incsearch` and `hlsearch`). Esc cancels the search and puts the
  view back where it was. All matches are black on cyan and the current match
  is black on magenta. Both are underlined, so they show without color too.
- An empty pattern (`/` then Enter) repeats the last pattern in the new direction.
- The `i` (ignore case) option changes the comparison only. It does not change
  searches; use `\c` for that.
- If the match is past the right edge, the view scrolls horizontally to show it.

Search line editing keys:

| Key | Effect |
|---|---|
| Enter | search |
| Esc (or `Ctrl-[`) | cancel |
| Backspace / `Ctrl-h` | delete a character; on an empty line, cancel |
| `Ctrl-w` | delete the word before the cursor |
| `Ctrl-u` | delete everything typed |
| ↑ / ↓ (or `Ctrl-p` / `Ctrl-n`) | older / newer entry from the history, limited to entries starting with what you typed |

The last 100 searches are kept, shared by `/` and `?`, for the rest of the
session. `:` commands have their own history.

#### Pattern syntax

Patterns use [Rust `regex` syntax](https://docs.rs/regex/latest/regex/#syntax)
with these vim additions:

| In the pattern | Meaning |
|---|---|
| `\c` anywhere | ignore case for the whole pattern |
| `\C` anywhere | match case (the default). If both appear, `\c` wins |
| `\<` / `\>` | start / end of a word |

Searches are case-sensitive by default, as in vim with `ignorecase` off.
Rust regex works like vim's "very magic" mode (`\v`): `( ) | + ? { }` are
special without a backslash, and you escape them to match them literally.
So vim's backslashed forms like `\(`, `\+` and `\{3}` match the literal characters
`(`, `+` and `{3}` here. Vim-only items such as `\zs`, `\%V` and `~` are not
supported. Common translations:

| Vim | Here |
|---|---|
| `/foo\c` | `/foo\c` (same) |
| `/\<word\>` | `/\<word\>` (same) |
| `/\(ab\)\+` | `/(ab)+` |
| `/foo\.c` | `/foo\.c` (same) |
| `/a\{3}` | `/a{3}` |
| `/a\+` | `/a+` |
| `/\d\+` | `/\d+` |

A bad pattern shows `E383: Invalid search string: … (reason)` and the view
stays where it was.

#### Messages

| Message | Meaning |
|---|---|
| `/foo  [3/12]` | jumped to match 3 of 12 in the active pane |
| `search hit BOTTOM, continuing at TOP` | wrapped past the end (or `TOP … BOTTOM` going backward) |
| `E486: Pattern not found: foo` | no match in the active pane; `(in right pane: Tab)` is added when the other pane has one |
| `E35: No previous regular expression` | `n`, `N` or an empty pattern before any search |
| `E383: Invalid search string: …` | the pattern is not valid regex |
| `E492: Not an editor command: …` | unknown `:` command |
| `E488: Trailing characters: …` | a `:` command got an argument it does not take |
| `E475: Invalid argument: … (left or right)` | `:sidebar` got something other than `left` or `right` |

Error messages are white on red. A message stays until the next key.

### Commands

`:` opens a command line on the bottom row in every view. It has the same
editing keys and its own history. Commands can be shortened as in vim:

| Command | Effect |
|---|---|
| `:h[elp]` | open the full key reference; Esc closes it (an argument is ignored) |
| `:sideb[ar]` | show / hide the sidebar, like `F1` |
| `:sidebar left`, `:sidebar right` | open the sidebar on that edge |
| `:noh[lsearch]` | hide search highlights until the next search, `n` or `N` |
| `:q[uit][!]` | close this view. A file opened from a folder returns to the list |
| `:qa[ll][!]`, `:quita[ll][!]` | quit the program from anywhere |

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
| `-L LABEL` (twice) | names to show for left and right, like `diff -L` |
| `--ai`, `--budget N`, `--hunk ID` | AI-friendly output (see [below](#for-ai-agents-and-llm-prompts)) |
| `-P NAME`, `--config FILE`, `--no-config` | profiles and config files |

## Using it from git

### A whole commit at once

```sh
mydiff git show                  # every file HEAD changed, in the folder view
mydiff git show abc123 -- src/   # one commit, limited to paths
mydiff git diff                  # uncommitted changes (work tree vs index)
mydiff git diff --staged         # what is staged (index vs HEAD)
mydiff git diff main             # work tree vs main
mydiff git diff main..topic      # also `main topic`, and `main...topic` (vs the merge base)
```

These need no git config. Revisions mean what they mean to `git diff`, and a
merge commit is shown against its first parent. The folder view lists only the
changed files, each by its full path (`docs/guide/intro.md`, not a collapsed
`docs/`), and a rename is one row, `> old -> new` (cyan) or `~ old -> new`
when the content changed too. Enter diffs the old path against the new one.
Piped or with `-f unified`, the output is a patch per file, with added and
deleted files shown in full against `/dev/null`. `-f json` gives each rename a
`renamed_from`. Comparison options (`-w`, `--include`, ...) apply as usual.

### As git's difftool

```sh
mydiff git setup >> ~/.gitconfig   # registers mydiff as git's difftool
git difftool                       # each changed file in the viewer (q = next file)
git difftool -d main               # every file changed since main, in the folder view
GIT_EXTERNAL_DIFF='mydiff git external' git diff   # git diff, printed by mydiff
```

Files are labelled by repository path (`a/src/x.rs`, `b/src/x.rs`). Added and
deleted files show against `/dev/null`. A JSON/YAML/CSV file that does not
parse (half-edited, say) falls back to a line diff instead of failing.
To make plain `git diff` always use mydiff, uncomment the `diff.external` lines
at the end of the snippet (`git diff --no-ext-diff` bypasses it). In that mode
mydiff exits 0 even when files differ, because git treats any other status as
a crash. Comparison options go before the git arguments, for example
`mydiff git external -w`.

## For AI agents and LLM prompts

```sh
mydiff --ai old.txt new.txt          # or -f ai; works for files, folders and git
MYDIFF_AI=1 mydiff git show          # export MYDIFF_AI=1 once in an agent's shell
mydiff --ai-help                     # the format, to paste into an agent's instructions
```

AI output is read-only, never opens the viewer and has no color. It starts with
a summary and a list of changed files (`M`/`A`/`D`/`R`/`B` plus `+added
-removed`), then each hunk with an id (`src/lib.rs#2`). Every line shows
**both** line numbers and its exact text, so an agent can quote it in an edit:

```text
@@ src/main.rs#1 -1,2 +1,3 @@
1 1   fn main() {
2   -     old();
  2 +     run();
  3 + }
```

Output stops after a line budget (`--budget N`, else `MYDIFF_AI_BUDGET`, else
2000; `0` = no limit) and ends with a `truncated:` line naming what was left
out. `--hunk <id>` (repeatable) shows only those hunks, in full, and
`--include <path>` narrows to files. Lockfiles and minified files are listed
but not expanded unless `--include` names them; whitespace-only changes and
pure renames are tagged. Under `git diff` with mydiff as external diff,
`MYDIFF_AI` is ignored so git's output stays a patch.

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
