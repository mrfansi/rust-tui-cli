# Architecture

The layout is split along **data flow**, not by type. There is no `models/`,
`views/`, `utils/` — each file owns one stage of getting data from an API onto a
screen, and the boundaries are what keep the thing extendable.

```
src/
├── main.rs        CLI definitions + dispatch. Nothing else.
├── commands.rs    One function per subcommand: resolve a client, call, print.
├── client.rs      HTTP. The only file that knows how the API reports an error.
├── config.rs      The profile store (a credentials file).
├── resource.rs    THE DOMAIN. Route, row, and what a status means.
├── output.rs      CLI printing + the small JSON helpers the TUI shares.
├── filter.rs      Substring-or-regex matching, shared by CLI and TUI.
└── tui/
    ├── mod.rs     Terminal + event loop. The only half that holds the store.
    ├── worker.rs  The network, on two threads. Knows only Req/Resp.
    ├── app.rs     State, and everything that decides.
    ├── keys.rs    Keypress → which decision.
    ├── render.rs  Drawing. Decides nothing.
    ├── form.rs    Modal forms: fields, focus, conditional visibility.
    ├── table.rs   Table navigation and column fitting.
    └── tests.rs   Keypress in, state out. No terminal, no server, no sleeps.
```

Outside `src/`, the files that exist because this is a template rather than a
project:

```
rename.sh                 renames a fresh copy, then deletes itself
tests/cli.rs              the CLI surface, exercised by running the binary
rust-toolchain.toml       stable + rustfmt + clippy, so `cargo fmt` agrees with CI
.github/workflows/ci.yml        lint · test on 3 OSes · MSRV
.github/workflows/template.yml  runs rename.sh and checks the result
.github/workflows/release.yml   a v* tag → five binaries, completions, man, SHA256SUMS
CLAUDE.md                 the rules below, for an agent working in a copy
```

`template.yml` and `rename.sh` are the two files that exist only because this is
a template, and `rename.sh` deletes both. Anything else added for the template's
benefit belongs in one of them, or it becomes something every copy inherits and
nobody remembers to remove.

## The rules that make it scale

**`render` never decides.** If drawing code needs to know whether something is
healthy, that's a function in `resource.rs`. The renderer asks; it does not
learn the API's vocabulary. Break this and every new screen re-implements the
same judgements slightly differently.

**`app` never draws, `keys` never stores.** `keys` maps a keypress to a method
on `App`; the method is the single definition of what that action does. The
actions menu holds `fn` pointers to those same methods, which is why a menu item
and a keybinding can never drift apart.

**Only `mod.rs` holds the `ProfileStore`.** It contains tokens. When the user
switches or adds a profile, the App sets `switch_to` / `add_profile` and the
event loop performs it. The drawing half never sees a credential.

**Two worker lanes.** `user` carries what was just pressed; `poll` carries the
background refresh. One lane would make a two-second tick queue ahead of a
delete the user is watching for. Within one request, a bulk fans out over
`std::thread::scope` bounded by `BULK_CONCURRENCY` — parallel because the round
trips are independent, bounded because 200 open connections at one host is an
attack, and scoped threads because the blocking client can simply be borrowed.

**The renderer owns the tab geometry.** `render_tabs` sets explicit padding and
divider and records each tab's span in `app.tab_spans`, so a click maps back to
a tab through arithmetic this repo controls rather than a guess at the widget's
defaults. `select_row_at` adds `TableState::offset()` for the same reason — skip
it and every click past the first screenful selects the wrong row.

**Overlays are drawn in the exact reverse of the order keys are consulted.**
`on_key` checks help → confirm → form → menu → picker; `ui` draws picker → menu
→ form → confirm → help. The overlay on top must be the one receiving the keys,
or the user types into a dialog hidden behind the one they can see. Nothing
fails today if the two drift — no path opens two overlays at once — which is
exactly why both lists are written down next to each other.

**The mouse cannot answer a question.** `on_mouse` returns immediately while any
overlay is open. A deletion that a stray click could confirm is not confirmed.

**A timeout is not a failure.** `gave_up_waiting()` separates "the server
refused" (a real failure) from "we stopped waiting" (the operation may well have
succeeded). Reporting the second as the first invites the user to do it twice.

**The filter matches the displayed row.** `resource::row()` is the source for
both the table and the filter, so searching for what you can see always works.

## Recipes

### Add a CLI subcommand

1. A variant in `Command` (or a sub-enum) in `main.rs`.
2. An arm in `main()`'s match.
3. A function in `commands.rs`.

Completions and the man page follow automatically.

### Add a TUI screen

1. A variant in `Screen` (`app.rs`).
2. Its label in `TABS` and its variant in `TAB_SCREENS` — same order.
3. Its index in `Screen::index()`.
4. Its keys in `render::screen_keys()` (this feeds both the help overlay and the
   status bar — do not write a second list).
5. An arm in `render::ui()` and one in `App::screen_key()`.

### Replace the demo resource

The first thing anyone does, and the one that touches the most files: `item`
appears on 125 lines across 13 of the 15 modules. Work outwards from the domain,
compiling as you go — each step below leaves the tree broken until the next one,
and the compiler names exactly what is left.

| Where | What to change |
|---|---|
| `resource.rs` | all of it: `PATH`, `HEADERS`, `row`, `id`, `health`, `create`, `new_body` |
| `main.rs` | `Command::Item`, `enum ItemCmd` and its variants' flags |
| `commands.rs` | `item_list` · `item_get` · `item_create` · `item_delete` |
| `tui/worker.rs` | `Req::Items` · `Resp::Items` and their arms in `handle()` |
| `tui/app.rs` | `Screen::Items` · `TABS` · `items` · `items_row` · `open_item_menu` · `FormKind::NewItem` · the fields in `open_new_form` |
| `tui/keys.rs` | `items_key` |
| `tui/render.rs` | `render_items` · the `Screen::Items` arm of `screen_keys` · the `min_widths` for your columns |
| `tui/tests.rs`, `client.rs`, `examples/fake_api.rs` | the fixtures and the `/items` paths |

**Do not do this with `sed`.** A case-aware substitution of `Item` was tried and
it does not work: `Item` is also `IntoIterator::Item` in `filter.rs`, ratatui's
`ListItem` in `render.rs`, and this repo's own `MenuItem` — which means "an item
of a menu", not an object of your domain. Three foreign meanings of one word, and
the first two do not even fail loudly in every case.

### Add a second resource

Copy `resource.rs` to `deploy.rs` (or whatever it is), then:

1. New `Req`/`Resp` variants in `worker.rs` and arms in `handle()`.
2. Its state on `App` (a `Vec<Value>` and a `TableState`).
3. A screen, per the recipe above.

The demo keeps one `resource` module because one is enough to show the shape.
Nothing about the structure assumes there is only one.

### Add an action to the menu

Write the method on `App` (in the `Actions` section), then add a `MenuItem`
pointing at it in `open_item_menu()`. If a keybinding should also trigger it,
call the same method from `keys.rs` — never duplicate the body.

### Grow the form

`Field::when("Kind", "db")` hides a field until another field holds a given
value; `validate()` only enforces required fields that are actually **visible**,
so a form can cover several shapes without becoming several forms. The reference
implementation adds a `step: u8` on `Field` to turn a long form into a wizard —
about 30 lines when you need it.

## What was left out, and when to add it

| Left out | Add it when |
|---|---|
| async runtime | blocking + scoped threads stops being enough; it already covers the bulk fan-out |
| logging framework | you need diagnostics from users' machines |
| a work queue for bulk | bulks are large AND per-item time is uneven, so chunk stragglers cost real time |
| `App` caching of `shown()` | a list gets big enough that rebuilding rows per frame shows |
| form wizard steps | a form has more fields than fit on a screen |
| CI dependency caching | a cold build costs more than the cache does; the suite currently runs in a tenth of a second |
| an auth trait | you need a second scheme in ONE binary. For one scheme it is a line in `send()`, and a trait with one implementation is a layer to read past |
| a configurable timeout | an operation other than `create` legitimately runs past 30 s. `post()` already takes a per-call timeout, so this is a flag, not a redesign |
| `cargo-generate` | `rename.sh` stops being enough — a template that has to ask more than the project's name |
| a `rename.sh` that renames the resource too | never, most likely: it would have to edit `resource.rs`, `worker.rs`, `app.rs` and `main.rs` in step, and the recipe above is the honest version of that work |
