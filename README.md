# rust-tui-cli

A starting point for a Rust tool that is **both** a scriptable CLI and an
interactive TUI over the same API — the architecture extracted from a
production tool that manages hundreds of resources across many hosts.

It builds, it has 66 passing tests on Linux, macOS and Windows, and
`cargo clippy --all-targets -- -D warnings` is clean. The demo domain is one
resource called "item"; replacing it is the whole job.

```
cargo run -- --help          # the CLI
cargo run                    # the TUI (no subcommand opens it)
cargo test
```

## Start a project from it

Click **Use this template** on GitHub, clone your copy, then:

```
./rename.sh fly-ctl
```

That renames the package and the binary, resets the changelog, regenerates the
lockfile, commits, and deletes itself. Nothing under `src/` is touched —
`APP_NAME` is `env!("CARGO_PKG_NAME")`, so the CLI name and the config directory
follow the package on their own. Pass `--no-commit` to review it first.

It deliberately does **not** rename the demo resource. That is a design
decision, not a substitution: `ARCHITECTURE.md` has the recipe.

## What you get

- **Multi-profile store** — many hosts, one default, tokens at `0600` in
  `$XDG_CONFIG_HOME/<binary>/profiles.json` (or `~/.config/…`). Written to a
  temporary file and renamed into place, so the mode is set before the token is,
  and an interrupted save cannot truncate the file. Add and switch from the CLI
  *or* the TUI.
- **Credentials from the environment** — `<APP>_URL` and `<APP>_TOKEN`, both or
  neither, used when no `--profile` was given. A CI run needs no file on disk.
- **One HTTP client** — typed `ApiError` carrying the status, a message pulled
  from whatever field your API uses, and `gave_up_waiting()` so a timeout is
  never reported as a failure.
- **CLI** — subcommands, a global `--profile` and `--json`, shell completions,
  and a man page, all generated from the same clap definitions.
- **TUI** — tabs, a filterable table (text *or* regex), marks + bulk actions,
  an actions menu, a modal form with conditional fields, a confirmation gate on
  anything destructive, a detail viewer, and a help overlay that cannot drift
  from the real keybindings.
- **Mouse** — click a tab or a row, right-click for the actions menu, wheel to
  scroll. Ignored entirely while a confirmation is up.
- **Networking off the UI thread** — two lanes (user actions, background
  polling) so a slow API never freezes the interface, and bulk actions fan out
  across scoped threads instead of one round trip at a time.
- **CI** — `fmt --check` and `clippy -D warnings` once, `test` on Linux, macOS
  and Windows, a build at the declared MSRV, and a job that runs `rename.sh` on
  a clean checkout so the template's own path stays tested.
- **Releases** — `git tag v0.1.0 && git push --tags` builds macOS (arm64, x64),
  Linux (gnu, musl) and Windows binaries and publishes them with a `SHA256SUMS`.

## Make it yours

1. `./rename.sh <your-name>` — see above.
2. Point `ApiClient` at your auth scheme (`src/client.rs` — one `send()`).
3. Rewrite `src/resource.rs` for your object. It owns the route, the row, and
   what a status *means*; nothing else knows any of that.
4. Adjust the subcommands in `src/main.rs` and the screens in `src/tui/app.rs`.
5. Rewrite this README's opening — `rename.sh` changes the title, not the prose.

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
| click / right-click / wheel | select a row or tab · actions menu · scroll |

## Deliberately not included

No async runtime (blocking reqwest on scoped threads covers even the bulk
fan-out, and far more simply), no logging framework, no config file beyond the
profile store, no multi-step form wizard, no auth scheme other than bearer (one
line in `send()`), and no `cargo-generate` template on top of `rename.sh`. Each
is a few lines away when you actually need it — ARCHITECTURE.md lists what
triggers each one.

## Licence

MIT.
