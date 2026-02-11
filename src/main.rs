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
    Following {
        /// Public key (npub, hex, or nprofile) to look up; defaults to your own
        npub: Option<String>,
    },

    /// List followers
    Followers {
        /// Public key (npub, hex, or nprofile) to look up; defaults to your own
        npub: Option<String>,
    },

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

    /// Watch for mentions, replies, and reactions in real-time
    Watch {
        /// Discord webhook URL (required)
        #[arg(long)]
        webhook: String,
        /// Target npub to watch (defaults to your own)
        #[arg(long)]
        npub: Option<String>,
        /// NIP-28 channel ID to watch (hex)
        #[arg(long)]
        channel: Option<String>,
    },

    /// Post a custom kind Nostr event
    Event {
        /// Event kind number
        #[arg(short, long)]
        kind: u16,
        /// Tags in "key,value" format (repeatable)
        #[arg(short, long)]
        tag: Vec<String>,
        /// Event content
        #[arg(short, long, default_value = "")]
        content: String,
    },

    /// Search for a vanity npub with a given prefix
    Vanity {
        /// Desired prefix after npub1
        prefix: String,
        /// Number of threads (default: CPU cores)
        #[arg(short, long)]
        threads: Option<usize>,
    },

    /// Search for a vanity npub with a given prefix
    Vanity {
        /// Desired prefix after npub1
        prefix: String,
        /// Number of threads (default: CPU cores)
        #[arg(short, long)]
        threads: Option<usize>,
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
        /// Lightning address (lud16)
        #[arg(long)]
        lud16: Option<String>,
        /// LNURL pay URL (lud06)
        #[arg(long)]
        lud06: Option<String>,
        /// NIP-05 identifier
        #[arg(long)]
        nip05: Option<String>,
        /// Banner image URL
        #[arg(long)]
        banner: Option<String>,
        /// Website URL
        #[arg(long)]
        website: Option<String>,
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
        /// Use NIP-04 (kind:4) instead of NIP-17
        #[arg(long)]
        nip04: bool,
    },
    /// Read received direct messages
    Read {
        /// Filter by sender npub (optional)
        npub: Option<String>,
    },
}

#[derive(Subcommand)]
enum ChannelAction {
    /// Create a new channel (kind:40)
    Create {
        /// Channel name
        #[arg(long)]
        name: String,
        /// Channel description
        #[arg(long)]
        about: Option<String>,
        /// Channel picture URL
        #[arg(long)]
        picture: Option<String>,
    },
    /// Edit channel metadata (kind:41)
    Edit {
        /// Channel ID (hex or note1...)
        id: String,
        /// New channel name
        #[arg(long)]
        name: String,
        /// New channel description
        #[arg(long)]
        about: Option<String>,
        /// New channel picture URL
        #[arg(long)]
        picture: Option<String>,
    },
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
                lud16,
                lud06,
                nip05,
                banner,
                website,
            } => {
                commands::profile::set(
                    name.as_deref(),
                    display_name.as_deref(),
                    about.as_deref(),
                    picture.as_deref(),
                    lud16.as_deref(),
                    lud06.as_deref(),
                    nip05.as_deref(),
                    banner.as_deref(),
                    website.as_deref(),
                )
                .await?
            }
        },
        Commands::Follow { npub } => commands::follow::follow(&npub).await?,
        Commands::Unfollow { npub } => commands::follow::unfollow(&npub).await?,
        Commands::Following { npub } => commands::follow::following(npub.as_deref()).await?,
        Commands::Followers { npub } => commands::follow::followers(npub.as_deref()).await?,
        Commands::React { note_id, emoji } => {
            commands::react::run(&note_id, &emoji).await?
        }
        Commands::Dm { action } => match action {
            DmAction::Send { npub, message, nip04 } => {
                commands::dm::send(&npub, &message, nip04).await?
            }
            DmAction::Read { npub } => commands::dm::read(npub.as_deref()).await?,
        },
        Commands::Zap {
            target,
            amount,
            message,
        } => commands::zap::run(&target, amount, message.as_deref()).await?,
        Commands::Channel { action } => match action {
            ChannelAction::Create {
                name,
                about,
                picture,
            } => {
                commands::channel::create(&name, about.as_deref(), picture.as_deref()).await?
            }
            ChannelAction::Edit {
                id,
                name,
                about,
                picture,
            } => {
                commands::channel::edit(&id, &name, about.as_deref(), picture.as_deref()).await?
            }
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
        Commands::Event { kind, tag, content } => {
            commands::event::run(kind, tag, &content).await?
        }
        Commands::Watch { webhook, npub, channel } => {
            commands::watch::run(&webhook, npub.as_deref(), channel.as_deref()).await?
        }
        Commands::Vanity { prefix, threads } => {
            commands::vanity::run(&prefix, threads)?
        }
        Commands::Vanity { prefix, threads } => {
            commands::vanity::run(&prefix, threads)?
        }
    }

    Ok(())
}
