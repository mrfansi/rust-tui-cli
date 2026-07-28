# Changelog

Notable changes, newest first. Format follows [Keep a Changelog]; versions
follow [Semantic Versioning].

## [Unreleased]

### Added

- `tests/cli.rs`: the CLI surface is exercised by running the binary — argument
  parsing, `--json`, the wording of the errors, and the exit codes a script
  depends on. Every test points `XDG_CONFIG_HOME` at a temporary directory.
- A test that runs `Cli::command().debug_assert()`, so a malformed clap
  definition fails in CI rather than in front of whoever adds a subcommand.
- Releases carry the shell completions and the man page alongside the binaries.
- `rename.sh`: renames a fresh copy of the template and deletes itself. Covered
  by a CI job, so the path a new user takes is tested rather than assumed.
- Credentials from the environment: `<APP>_URL` and `<APP>_TOKEN` are used when
  both are set and no `--profile` was given, so CI runs need no credentials file
  on disk.
- `XDG_CONFIG_HOME` is honoured when it names an absolute directory.
- Release workflow: a `v*` tag builds macOS (arm64, x64), Linux (gnu, musl) and
  Windows binaries, and publishes them with a `SHA256SUMS`.
- `rust-version = "1.88"`, checked by a CI job that builds with that toolchain.
- CI runs the test suite on Linux, macOS and Windows.

### Fixed

- Mouse capture is turned off when the TUI panics. `ratatui::init()`'s hook
  leaves raw mode and the alternate screen, but the capture was enabled by this
  crate, so nothing switched it off — leaving the user's shell answering every
  click with an escape sequence until they ran `reset`.
- The profile store is written to a temporary file created with mode `0600` and
  then renamed into place. It was written with the default umask and chmod'ed
  afterwards, leaving tokens world-readable in between, and an interrupted write
  could truncate the file that holds them.

## [0.1.0]

- CLI + TUI boilerplate: profiles, tables, dashboard, forms, mouse support,
  parallel bulk actions.

[Keep a Changelog]: https://keepachangelog.com/en/1.1.0/
[Semantic Versioning]: https://semver.org/spec/v2.0.0.html
