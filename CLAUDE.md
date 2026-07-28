# CLAUDE.md

A Rust binary that is both a scriptable CLI and an interactive TUI over one API.

## Commands

```
cargo run -- --help          the CLI
cargo run                    the TUI (no subcommand opens it)
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

All three of `fmt --check`, `clippy -D warnings`, and `test` must pass before any
change is done. CI runs exactly those, on Linux, macOS, and Windows.

## Layout

Split along **data flow**, not by type — there is no `models/`, `views/`,
`utils/`. `ARCHITECTURE.md` has the full map and the reasoning; the short version:

| | |
|---|---|
| `main.rs` | clap definitions + dispatch, nothing else |
| `commands.rs` | one function per subcommand |
| `client.rs` | HTTP; the only file that knows how the API reports an error |
| `config.rs` | the profile store — a credentials file |
| `resource.rs` | **the domain**: route, row, and what a status means |
| `output.rs` / `filter.rs` | CLI printing; matching shared by CLI and TUI |
| `tui/` | `mod` loop · `worker` network · `app` state · `keys` input · `render` drawing · `form` · `table` |

## Rules that must not be broken

- **`render` never decides.** A judgement about the domain (is this healthy?) is a
  function in `resource.rs`. The renderer asks; it never learns the API's vocabulary.
- **`app` never draws, `keys` never stores.** `keys` maps a keypress to a method on
  `App`; that method is the single definition of the action. The actions menu holds
  `fn` pointers to those same methods, so a menu item and a keybinding cannot drift.
- **Only `tui/mod.rs` holds the `ProfileStore`.** It contains tokens. The drawing
  half signals intent (`switch_to`, `add_profile`); the event loop performs it.
- **A timeout is not a failure.** `gave_up_waiting()` separates "the server refused"
  from "we stopped waiting". Reporting the second as the first makes users retry a
  destructive operation that already succeeded.
- **The filter matches the displayed row.** `resource::row()` feeds both the table
  and the filter, so searching for what you can see always works.
- **Never derive `Debug` on anything holding a token.** `ApiClient`'s is hand-written
  and redacts; a derived one would print the token into every panic and log line.

## Extending

`ARCHITECTURE.md` has step-by-step recipes for: a new CLI subcommand, a new TUI
screen, a second resource, a new menu action, and growing the form. Follow them
rather than inventing a parallel structure — each recipe exists because the
alternative drifts.

Tests live beside the code they test (`#[cfg(test)] mod tests`), except the TUI's,
which are in `tui/tests.rs`: keypress in, state out, no terminal and no sleeps.
Name a test after the behaviour it protects, not the function it calls.

## This repo is a template

`rename.sh` renames a fresh copy. It is deleted by its own run — if it is still
here, this checkout has not been renamed yet.
