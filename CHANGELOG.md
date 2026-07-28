# Changelog

Notable changes, newest first. Format follows [Keep a Changelog]; versions
follow [Semantic Versioning].

## [Unreleased]

### Added

- **Update — the U that was missing from CRUD.** `client.patch`,
  `resource::update`, an `item set` subcommand, and an "Edit…" action bound to
  `e` in the TUI. Create, read and delete existed at every layer; update existed
  at none, even though `profile set` proved the repo already knew updates
  matter. `PATCH`, not `PUT`: a form showing four of an object's twelve fields
  would otherwise send four and erase the other eight.
- `Field` remembers the value it opened with, so an edit sends only what
  actually changed. An absent field and an empty one stay different requests —
  "leave the owner alone" and "clear the owner" must not collapse into one.
- Pagination is now named in "what was left out", with the reason it is not
  demonstrated: four incompatible shapes, and showing one teaches the wrong one
  to everyone else.

- A "Replace the demo resource" recipe in `ARCHITECTURE.md`: a file-by-file
  table of the 125 lines the name `item` reaches, and the evidence for why a
  `sed` cannot do it — `Item` is also `IntoIterator::Item`, ratatui's `ListItem`,
  and this repo's own `MenuItem`. Replacing the demo is the first thing anyone
  does here and it was the least documented.
- A note that `rename.sh` needs Git Bash on Windows.

## [0.1.0] — 2026-07-28

The first release. Everything below shipped in it: the "Fixed" entries repair
faults in code that had never been published, so nobody was ever exposed to them
— they are listed because the reasoning is worth keeping, not because anyone
needs to upgrade past them.

### Added

- `tests/cli.rs`: the CLI surface is exercised by running the binary — argument
  parsing, `--json`, the wording of the errors, and the exit codes a script
  depends on. Every test points `XDG_CONFIG_HOME` at a temporary directory.
- A test that runs `Cli::command().debug_assert()`, so a malformed clap
  definition fails in CI rather than in front of whoever adds a subcommand.
- Releases carry the shell completions and the man page alongside the binaries.
- `examples/fake_api.rs`: a local server answering the shape `resource.rs`
  expects, so a fresh clone can be seen working before there is an API to point
  it at. `httpmock` is a dev-dependency, so it costs a release build nothing.
- The release workflow refuses a tag that disagrees with `version` in
  `Cargo.toml`, before it builds anything.
- `rename.sh`: renames a fresh copy of the template and deletes itself, along
  with the workflow that tests it. Covered by that workflow, so the path a new
  user takes is tested rather than assumed.
- Credentials from the environment: `<APP>_URL` and `<APP>_TOKEN` are used when
  both are set and no `--profile` was given, so CI runs need no credentials file
  on disk.
- `XDG_CONFIG_HOME` is honoured when it names an absolute directory.
- Release workflow: a `v*` tag builds macOS (arm64, x64), Linux (gnu, musl) and
  Windows binaries, and publishes them with a `SHA256SUMS`.
- `rust-version = "1.88"`, checked by a CI job that builds with that toolchain.
- CI runs the test suite on Linux, macOS and Windows.

### Fixed

- `httpmock` moved to 0.8: `Mock::assert_hits` is deprecated there in favour of
  `assert_calls`, and `-D warnings` makes a deprecation a build failure.
- A table cell too wide for its column is cut with an ellipsis. The cut was made
  at the whole table's width, so a fixed-width column was left to the widget,
  which clips without a mark — `api-billing-prod` appeared as `api-billing-`, a
  name that looks complete and is not.
- A modifier makes a keypress a command, not a character. `Ctrl-U` in a filter
  box or a form field used to insert a `u`.
- Overlays are drawn in the exact reverse of the order `on_key` consults them,
  so the dialog on top is always the one receiving the keys. The menu was drawn
  over the form but ranked below it — unreachable today, and a trap for the next
  screen someone adds.
- The workflow that tests `rename.sh` lives in its own file and is deleted by
  the rename. As a job in `ci.yml` it was inherited by every project made from
  this template, where it could only fail — the script it runs has deleted
  itself by then, so a new project's first push went red for no reason.
- Mouse capture is turned off when the TUI panics. `ratatui::init()`'s hook
  leaves raw mode and the alternate screen, but the capture was enabled by this
  crate, so nothing switched it off — leaving the user's shell answering every
  click with an escape sequence until they ran `reset`.
- The profile store is written to a temporary file created with mode `0600` and
  then renamed into place. It was written with the default umask and chmod'ed
  afterwards, leaving tokens world-readable in between, and an interrupted write
  could truncate the file that holds them.

- The CLI + TUI foundation itself: profiles, tables, dashboard, forms, mouse
  support, parallel bulk actions.
- Shell completions, a man page, and prebuilt binaries for macOS (arm64, x64),
  Linux (gnu, musl) and Windows, published with a `SHA256SUMS`.

[Keep a Changelog]: https://keepachangelog.com/en/1.1.0/
[Semantic Versioning]: https://semver.org/spec/v2.0.0.html
