# rust-tui-cli

A starting point for a Rust tool that is **both** a scriptable CLI and an
interactive TUI over the same API — the architecture extracted from a
production tool that manages hundreds of resources across many hosts.

It builds, it has 51 passing tests, and `cargo clippy` is clean. The demo domain
is one resource called "item"; replacing it is the whole job.

```
cargo run -- --help          # the CLI
cargo run                    # the TUI (no subcommand opens it)
cargo test
```

## What you get

- **Multi-profile store** — many hosts, one default, tokens at `0600` in
  `~/.config/<binary>/profiles.json`. Add and switch from the CLI *or* the TUI.
- **One HTTP client** — typed `ApiError` carrying the status, a message pulled
  from whatever field your API uses, and `gave_up_waiting()` so a timeout is
  never reported as a failure.
- **CLI** — subcommands, a global `--profile` and `--json`, shell completions,
  and a man page, all generated from the same clap definitions.
- **TUI** — tabs, a filterable table (text *or* regex), marks + bulk actions,
  an actions menu, a modal form with conditional fields, a confirmation gate on
  anything destructive, a detail viewer, and a help overlay that cannot drift
  from the real keybindings.
- **Networking off the UI thread** — two lanes (user actions, background
  polling) so a slow API never freezes the interface.

## Make it yours

1. Rename the package in `Cargo.toml`. `APP_NAME` is `env!("CARGO_PKG_NAME")`,
   so the CLI name and the config directory follow.
2. Point `ApiClient` at your auth scheme (`src/client.rs` — one `send()`).
3. Rewrite `src/resource.rs` for your object. It owns the route, the row, and
   what a status *means*; nothing else knows any of that.
4. Adjust the subcommands in `src/main.rs` and the screens in `src/tui/app.rs`.

`ARCHITECTURE.md` has the module map and the recipe for adding a screen or a
second resource.

## Keys

| Key | |
|---|---|
| `1-9` / `Tab` / `←→` | switch tab |
| `s` | profile list (`Enter` switch, `a` add) |
| `r` | refresh |
| `?` | help |
| `q` / `Ctrl-C` | quit |
| `/` | filter (text or regex) |
| `Enter` | detail |
| `Space` | actions menu |
| `n` | new |
| `v` / `V` | mark / mark everything shown |
| `x` | delete (marked rows, or the one under the cursor) |

## Deliberately not included

No async runtime (blocking reqwest on worker threads is enough and far simpler),
no logging framework, no mouse support, no config file beyond the profile store,
no multi-step form wizard, no CI workflow. Each is a few lines away when you
actually need it — see ARCHITECTURE.md.

## Licence

MIT.
