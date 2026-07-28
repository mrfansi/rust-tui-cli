#!/bin/sh
# Rename this template to your own project, then delete itself.
#
#   ./rename.sh fly-ctl              rename, then commit
#   ./rename.sh fly-ctl --no-commit  rename only, leave it staged for you
#
# Only three files carry the name: Cargo.toml (package and [[bin]]), README.md,
# and CHANGELOG.md. Nothing under src/ does — APP_NAME is env!("CARGO_PKG_NAME"),
# so the CLI name and the config directory follow the package automatically.
#
# What this does NOT do is rename the demo resource ("item") to your domain
# object. That is a design decision, not a text substitution: see the "Add a
# second resource" recipe in ARCHITECTURE.md.

set -eu

usage() {
	echo "usage: ./rename.sh <new-name> [--no-commit]" >&2
	echo "  <new-name> must match ^[a-z][a-z0-9-]*\$ — it becomes a crate name," >&2
	echo "  a binary name, and a directory under \$XDG_CONFIG_HOME." >&2
	exit 2
}

NEW=""
COMMIT=yes
for arg in "$@"; do
	case "$arg" in
	--no-commit) COMMIT=no ;;
	-h | --help) usage ;;
	-*) echo "rename.sh: unknown option '$arg'" >&2 && usage ;;
	*)
		[ -z "$NEW" ] || { echo "rename.sh: more than one name given" >&2 && usage; }
		NEW="$arg"
		;;
	esac
done
[ -n "$NEW" ] || usage

# The same alphabet the tool enforces for profile names, and for the same
# reason: this string ends up in a path and on a command line.
if ! echo "$NEW" | grep -Eq '^[a-z][a-z0-9-]*$'; then
	echo "rename.sh: '$NEW' is not a valid name (lowercase letters, digits and '-', starting with a letter)" >&2
	exit 1
fi

cd "$(dirname "$0")"

# Read the current name rather than assuming it, so a half-finished rename can
# be finished rather than producing 'rust-tui-cli-fly-ctl'.
OLD=$(sed -n 's/^name = "\(.*\)"$/\1/p' Cargo.toml | head -1)
[ -n "$OLD" ] || { echo "rename.sh: could not read the package name from Cargo.toml" >&2 && exit 1; }
if [ "$OLD" = "$NEW" ]; then
	echo "rename.sh: already named '$NEW' — nothing to do."
	exit 0
fi

# A dirty tree would go into the rename commit along with the rename. Only
# checked when we are the one committing.
if [ "$COMMIT" = yes ] && [ -d .git ] && [ -n "$(git status --porcelain)" ]; then
	echo "rename.sh: working tree is not clean. Commit or stash first, or pass --no-commit." >&2
	exit 1
fi

# sed -i is spelled differently on BSD and GNU; a temp file works on both.
replace() {
	file=$1
	sed "s/$OLD/$NEW/g" "$file" >"$file.rename.tmp" && mv "$file.rename.tmp" "$file"
}

replace Cargo.toml
replace README.md

# The CI badge points at the template's own repository. Substituting the name
# would rewrite only half that URL and leave the original owner, producing a
# badge for a repository nobody owns — worse than no badge. Dropped; add your
# own once yours is pushed.
sed '/actions\/workflows\/ci\.yml\/badge\.svg/d' README.md >README.md.rename.tmp &&
	mv README.md.rename.tmp README.md

# The template's own description and keywords ("boilerplate", "template") stop
# being true the moment this is a real project, and all three fields matter only
# to crates.io. Dropped rather than replaced with a placeholder: cargo names
# exactly what is missing if this is ever published, whereas a TODO nobody is
# obliged to read is the wrong description with an extra step.
sed '/^description = /d; /^keywords = /d; /^categories = /d' Cargo.toml >Cargo.toml.rename.tmp &&
	mv Cargo.toml.rename.tmp Cargo.toml
# Reset rather than renamed. The template's history is not the new project's
# history, and a first release that claims to have "fixed" a bug it never had
# is a changelog nobody will trust twice.
#
# Guarded with `if` rather than `&&`: under `set -e` a false `&&` chain is a
# non-zero exit, which would abort the rename half-done.
if [ -f CHANGELOG.md ]; then
	cat >CHANGELOG.md <<EOF
# Changelog

Notable changes, newest first. Format follows [Keep a Changelog]; versions
follow [Semantic Versioning].

## [Unreleased]

- Started from the rust-tui-cli template.

[Keep a Changelog]: https://keepachangelog.com/en/1.1.0/
[Semantic Versioning]: https://semver.org/spec/v2.0.0.html
EOF
fi

# Cargo.lock names the package too. Re-resolving is safer than a substitution,
# which would also rewrite a dependency that happened to share the old name.
if command -v cargo >/dev/null 2>&1; then
	cargo update --workspace --quiet
fi

# The template's own CI job renames a checkout and asserts the result. Here it
# could only fail: the rename has happened and the script below is about to
# delete itself. Removed with the script, not left to go red on the first push.
rm -f .github/workflows/template.yml

rm -f "$0"

echo "Renamed $OLD -> $NEW."

if [ "$COMMIT" = yes ] && [ -d .git ]; then
	git add -A
	git commit -q -m "Rename to $NEW"
	echo "Committed."
fi

cat <<EOF

Next, in order:

  1. src/client.rs   point ApiClient at your auth scheme (one send()).
  2. src/resource.rs rewrite for your object: route, row, what a status means.
  3. src/main.rs     adjust the subcommands.
  4. README.md       the title is renamed; the prose still describes a template.

  cargo test

ARCHITECTURE.md has the recipes for a new screen and a second resource.
EOF
