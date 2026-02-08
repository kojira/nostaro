use clap::{Parser, Subcommand};
use nostaro::commands;

#[derive(Parser)]
#[command(name = "nostaro", version, about = "A Nostr CLI tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize nostaro with a new or existing keypair
    Init,

    /// Post a text note to Nostr (kind:1)
    Post {
        /// The message to post
        message: String,
    },

    /// Reply to a note (kind:1 with e/p tags)
    Reply {
        /// Note ID to reply to (note1... or hex)
        note_id: String,
        /// Reply message
        message: String,
    },

    /// Repost a note (kind:6)
    Repost {
        /// Note ID to repost (note1... or hex)
        note_id: String,
    },

    /// View your timeline
    Timeline {
        /// Maximum number of notes to fetch
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },

    /// Search notes (NIP-50)
    Search {
        /// Search query
        query: String,
        /// Maximum number of results
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },

    /// View or set a Nostr profile
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },

    /// Follow a user (kind:3)
    Follow {
        /// Public key (npub or hex) to follow
        npub: String,
    },

    /// Unfollow a user (kind:3)
    Unfollow {
        /// Public key (npub or hex) to unfollow
        npub: String,
    },

    /// List users you're following
    Following,

    /// React to a note (kind:7)
    React {
        /// Note ID (note1... or hex)
        note_id: String,
        /// Reaction emoji (default: ⚡)
        #[arg(default_value = "\u{26A1}")]
        emoji: String,
    },

    /// Direct messages (NIP-44/NIP-17)
    Dm {
        #[command(subcommand)]
        action: DmAction,
    },

    /// Send a zap (NIP-57)
    Zap {
        /// Target npub or note ID
        target: String,
        /// Amount in satoshis
        amount: u64,
        /// Optional message
        #[arg(short, long)]
        message: Option<String>,
    },

    /// Channel commands (NIP-28)
    Channel {
        #[command(subcommand)]
        action: ChannelAction,
    },

    /// Upload a file via Blossom (default) or NIP-96
    Upload {
        /// Path to the file to upload
        file: String,
        /// Custom upload server URL
        #[arg(long)]
        server: Option<String>,
        /// Use NIP-96 instead of Blossom
        #[arg(long)]
        nip96: bool,
    },

    /// Manage the local cache
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },

    /// Manage relay connections
    Relay {
        #[command(subcommand)]
        action: RelayAction,
    },
}

#[derive(Subcommand)]
enum ProfileAction {
    /// Show a Nostr profile
    Show {
        /// Public key (npub or hex) to look up; defaults to your own
        #[arg(short = 'p', long)]
        pubkey: Option<String>,
    },
    /// Set your profile metadata (kind:0)
    Set {
        /// Name (username)
        #[arg(long)]
        name: Option<String>,
        /// Display name
        #[arg(long)]
        display_name: Option<String>,
        /// About / bio
        #[arg(long)]
        about: Option<String>,
        /// Profile picture URL
        #[arg(long)]
        picture: Option<String>,
    },
}

#[derive(Subcommand)]
enum DmAction {
    /// Send a direct message
    Send {
        /// Recipient npub or hex pubkey
        npub: String,
        /// Message to send
        message: String,
    },
    /// Read received direct messages
    Read {
        /// Filter by sender npub (optional)
        npub: Option<String>,
    },
}

#[derive(Subcommand)]
enum ChannelAction {
    /// List channels
    List,
    /// Read channel messages
    Read {
        /// Channel ID (hex or note1...)
        id: String,
    },
    /// Post a message to a channel
    Post {
        /// Channel ID (hex or note1...)
        id: String,
        /// Message to post
        message: String,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    /// Clear all cached data
    Clear,
    /// Show cache statistics
    Stats,
}

#[derive(Subcommand)]
enum RelayAction {
    /// Add a relay
    Add {
        /// Relay WebSocket URL (e.g. wss://relay.damus.io)
        url: String,
    },
    /// Remove a relay
    Remove {
        /// Relay WebSocket URL to remove
        url: String,
    },
    /// List all configured relays
    List,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => commands::init::run().await?,
        Commands::Post { message } => commands::post::run(&message).await?,
        Commands::Reply { note_id, message } => {
            commands::reply::run(&note_id, &message).await?
        }
        Commands::Repost { note_id } => commands::repost::run(&note_id).await?,
        Commands::Timeline { limit } => commands::timeline::run(limit).await?,
        Commands::Search { query, limit } => commands::search::run(&query, limit).await?,
        Commands::Profile { action } => match action {
            ProfileAction::Show { pubkey } => {
                commands::profile::show(pubkey.as_deref()).await?
            }
            ProfileAction::Set {
                name,
                display_name,
                about,
                picture,
            } => {
                commands::profile::set(
                    name.as_deref(),
                    display_name.as_deref(),
                    about.as_deref(),
                    picture.as_deref(),
                )
                .await?
            }
        },
        Commands::Follow { npub } => commands::follow::follow(&npub).await?,
        Commands::Unfollow { npub } => commands::follow::unfollow(&npub).await?,
        Commands::Following => commands::follow::following().await?,
        Commands::React { note_id, emoji } => {
            commands::react::run(&note_id, &emoji).await?
        }
        Commands::Dm { action } => match action {
            DmAction::Send { npub, message } => {
                commands::dm::send(&npub, &message).await?
            }
            DmAction::Read { npub } => commands::dm::read(npub.as_deref()).await?,
        },
        Commands::Zap {
            target,
            amount,
            message,
        } => commands::zap::run(&target, amount, message.as_deref()).await?,
        Commands::Channel { action } => match action {
            ChannelAction::List => commands::channel::list().await?,
            ChannelAction::Read { id } => commands::channel::read(&id).await?,
            ChannelAction::Post { id, message } => {
                commands::channel::post(&id, &message).await?
            }
        },
        Commands::Upload {
            file,
            server,
            nip96,
        } => commands::upload::run(&file, server.as_deref(), nip96).await?,
        Commands::Cache { action } => match action {
            CacheAction::Clear => commands::cache::clear().await?,
            CacheAction::Stats => commands::cache::stats().await?,
        },
        Commands::Relay { action } => match action {
            RelayAction::Add { url } => commands::relay::add(&url).await?,
            RelayAction::Remove { url } => commands::relay::remove(&url).await?,
            RelayAction::List => commands::relay::list().await?,
        },
    }

    Ok(())
}
