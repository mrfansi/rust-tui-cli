//! Entry point. Parses the CLI, resolves a profile, dispatches — and nothing else.
//!
//! Keeping the argument definitions here and the work in `commands` means the
//! shape of the CLI is readable in one file, and `--help`, the completion script
//! and the man page all come from these same definitions.

mod client;
mod commands;
mod config;
mod filter;
mod output;
mod resource;
mod tui;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use config::ProfileStore;

/// The binary's name, used for the CLI name and the config directory.
/// Rename the package in Cargo.toml and both follow.
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Parser)]
#[command(
    name = APP_NAME,
    version,
    about = "A CLI + TUI starting point: profiles, tables, and a dashboard"
)]
struct Cli {
    /// Target profile (default: the one marked default)
    #[arg(long, global = true)]
    profile: Option<String>,

    /// Print the API's raw JSON instead of a table (read-only commands)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Manage the hosts this tool can talk to
    #[command(subcommand)]
    Profile(ProfileCmd),
    /// Manage items (the demo resource — replace with your own)
    #[command(subcommand)]
    Item(ItemCmd),
    /// Interactive TUI (also what you get with no subcommand)
    Menu,
    /// Print a shell completion script (bash, zsh, fish, elvish, powershell)
    Completions {
        /// Target shell; omit to guess from $SHELL
        shell: Option<Shell>,
    },
    /// Print the man page (roff) to stdout
    Man,
}

#[derive(Subcommand)]
enum ProfileCmd {
    /// Add a profile (prompts for anything not given)
    Add {
        name: Option<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    /// List profiles
    List,
    /// Change a profile's URL or token
    Set {
        name: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    /// Make a profile the default
    Use { name: String },
    /// Remove a profile and its token
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ItemCmd {
    /// List items
    List {
        /// Keep only rows matching this text or regex
        #[arg(long)]
        filter: Option<String>,
    },
    /// Show one item
    Get { id: String },
    /// Create an item
    Create {
        name: String,
        #[arg(long, default_value = "app")]
        kind: String,
        #[arg(long)]
        owner: Option<String>,
        /// Only meaningful for --kind db
        #[arg(long)]
        image: Option<String>,
    },
    /// Change an item's name or owner
    ///
    /// Named `set` to match `profile set`: one verb for "change what exists"
    /// across the whole CLI, rather than `set` here and `edit` there.
    Set {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        owner: Option<String>,
    },
    /// Delete an item
    Delete {
        id: String,
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    output::set_json_output(cli.json);

    let store = ProfileStore::new(ProfileStore::default_path());

    match cli.command {
        Some(Command::Profile(cmd)) => match cmd {
            ProfileCmd::Add { name, url, token } => commands::profile_add(&store, name, url, token),
            ProfileCmd::List => commands::profile_list(&store),
            ProfileCmd::Set { name, url, token } => {
                commands::profile_set(&store, &name, url, token)
            }
            ProfileCmd::Use { name } => commands::profile_use(&store, &name),
            ProfileCmd::Remove { name, yes } => commands::profile_remove(&store, &name, yes),
        },

        Some(Command::Item(cmd)) => {
            let client = commands::resolve_client(&store, &cli.profile)?;
            match cmd {
                ItemCmd::List { filter } => commands::item_list(&client, filter),
                ItemCmd::Get { id } => commands::item_get(&client, &id),
                ItemCmd::Create {
                    name,
                    kind,
                    owner,
                    image,
                } => commands::item_create(&client, name, kind, owner, image),
                ItemCmd::Set { id, name, owner } => commands::item_set(&client, &id, name, owner),
                ItemCmd::Delete { id, yes } => commands::item_delete(&client, &id, yes),
            }
        }

        Some(Command::Completions { shell }) => {
            // Guessing from $SHELL means the documented one-liner works without
            // the user having to name their own shell to their own shell.
            let shell = shell.or_else(Shell::from_env).unwrap_or(Shell::Bash);
            clap_complete::generate(shell, &mut Cli::command(), APP_NAME, &mut std::io::stdout());
            Ok(())
        }

        Some(Command::Man) => {
            clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout())?;
            Ok(())
        }

        // No subcommand opens the TUI: the interactive half is the point of the
        // tool, not a mode you have to know the name of.
        Some(Command::Menu) | None => run_tui(&store, &cli.profile),
    }
}

fn run_tui(store: &ProfileStore, profile: &Option<String>) -> Result<()> {
    let client = commands::resolve_client(store, profile)?;
    let name = match profile {
        Some(n) => n.clone(),
        None => store.default().map(|p| p.name).unwrap_or_default(),
    };
    tui::run(store, client, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// clap checks its own definition, but only in a debug build and only when
    /// the offending path is actually run. This puts the check in the suite, so
    /// a duplicate short flag or a required argument sitting after an optional
    /// one is caught by CI rather than by whoever types the command next.
    ///
    /// Worth its four lines in a template above all: adding a subcommand is the
    /// first thing anyone does here, and it is where those mistakes are made.
    #[test]
    fn the_cli_definition_is_well_formed() {
        Cli::command().debug_assert();
    }
}
