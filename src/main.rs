use clap::{Parser, Subcommand};
use cliplan::config::Config;
use std::path::PathBuf;

/// ClipLAN -- passwordless LAN clipboard & file sharing.
#[derive(Debug, Parser)]
#[command(name = "cliplan", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to config.toml. Defaults to ~/.cliplan/config.toml
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Port to listen on (overrides config file)
    #[arg(long)]
    port: Option<u16>,

    /// Shared directory for file transfers (overrides config file)
    #[arg(long)]
    directory: Option<String>,

    /// Disable LAN discovery
    #[arg(long)]
    no_discovery: bool,

    /// Disable clipboard sync
    #[arg(long)]
    no_clipboard: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the ClipLAN server (default when no subcommand is given)
    Run,
    /// List paired devices (talks to a locally running instance)
    Devices,
    /// Show server status (talks to a locally running instance)
    Status,
    /// Review and approve/reject pending pairing requests
    Pair,
    /// Show recent clipboard history
    History,
    /// Print the resolved configuration
    Config,
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
}

fn load_config(cli: &Cli) -> anyhow::Result<Config> {
    let path = cli
        .config
        .clone()
        .unwrap_or_else(|| Config::default().data_dir_path().join("config.toml"));
    let mut cfg = Config::load(&path)?;

    if let Some(port) = cli.port {
        cfg.port = port;
    }
    if let Some(dir) = &cli.directory {
        cfg.shared_directory = dir.clone();
    }
    if cli.no_discovery {
        cfg.discovery = false;
    }
    if cli.no_clipboard {
        cfg.clipboard_sync = false;
    }

    // Persist the (possibly first-run default) config so `~/.cliplan/config.toml`
    // always reflects what's actually in effect and future edits have
    // something to start from.
    let _ = cfg.save(&path);

    Ok(cfg)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let config = load_config(&cli)?;

    match cli.command.unwrap_or(Command::Run) {
        Command::Run => cliplan::server::run(config).await?,
        Command::Devices => cli_client::devices(&config).await?,
        Command::Status => cli_client::status(&config).await?,
        Command::Pair => cli_client::pair(&config).await?,
        Command::History => cli_client::history(&config).await?,
        Command::Config => println!("{}", toml::to_string_pretty(&config)?),
    }

    Ok(())
}

/// Small blocking-free HTTP client used by the CLI subcommands to talk to
/// an already-running ClipLAN instance over loopback. These are
/// conveniences for the person sitting at the host machine -- the same
/// admin actions are also reachable from the dashboard if opened on the
/// host itself.
mod cli_client {
    use cliplan::config::Config;
    use std::io::Write;

    fn base_url(config: &Config) -> String {
        format!("http://127.0.0.1:{}", config.port)
    }

    fn client_error_hint(e: &reqwest::Error) -> String {
        if e.is_connect() {
            "Could not reach a running ClipLAN server on this port. Is `cliplan` running?".to_string()
        } else {
            e.to_string()
        }
    }

    pub async fn status(config: &Config) -> anyhow::Result<()> {
        let url = format!("{}/api/status", base_url(config));
        let resp = reqwest::get(&url).await.map_err(|e| anyhow::anyhow!(client_error_hint(&e)))?;
        let body: serde_json::Value = resp.json().await?;
        println!("{}", serde_json::to_string_pretty(&body)?);
        Ok(())
    }

    pub async fn devices(config: &Config) -> anyhow::Result<()> {
        // Device listing requires a paired device's token; the CLI itself
        // isn't paired, so we fall back to showing pending requests plus a
        // hint. Pending-request review lives in `pair` below.
        println!("Use the dashboard (as a paired device) to view the full device list.");
        println!("Showing pending pairing requests instead:\n");
        pair(config).await
    }

    pub async fn pair(config: &Config) -> anyhow::Result<()> {
        let url = format!("{}/api/devices/pairing/pending", base_url(config));
        let resp = reqwest::get(&url).await.map_err(|e| anyhow::anyhow!(client_error_hint(&e)))?;
        let pending: Vec<serde_json::Value> = resp.json().await?;

        if pending.is_empty() {
            println!("No pending pairing requests.");
            let session_url = format!("{}/api/devices/pairing/session", base_url(config));
            if let Ok(resp) = reqwest::get(&session_url).await {
                if let Ok(session) = resp.json::<serde_json::Value>().await {
                    println!("\nCurrent pairing code: {}", session["code"]);
                    println!("Pair URL: {}", session["pair_url"]);
                }
            }
            return Ok(());
        }

        for (i, req) in pending.iter().enumerate() {
            println!(
                "[{}] {}  fingerprint: {}  requested: {}",
                i + 1,
                req["device_name"].as_str().unwrap_or("?"),
                req["fingerprint"].as_str().unwrap_or("?"),
                req["created_at"].as_str().unwrap_or("?"),
            );
        }

        print!("\nApprove which # (or 'q' to quit, 'r <#>' to reject): ");
        std::io::stdout().flush().ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.eq_ignore_ascii_case("q") || input.is_empty() {
            return Ok(());
        }

        let client = reqwest::Client::new();
        if let Some(rest) = input.strip_prefix('r') {
            let idx: usize = rest.trim().parse().unwrap_or(0);
            if let Some(req) = pending.get(idx.saturating_sub(1)) {
                let id = req["id"].as_str().unwrap_or_default();
                let url = format!("{}/api/devices/pairing/{}/reject", base_url(config), id);
                client.post(&url).send().await?;
                println!("Rejected.");
            }
        } else if let Ok(idx) = input.parse::<usize>() {
            if let Some(req) = pending.get(idx.saturating_sub(1)) {
                let id = req["id"].as_str().unwrap_or_default();
                let url = format!("{}/api/devices/pairing/{}/approve", base_url(config), id);
                client.post(&url).send().await?;
                println!("Approved.");
            }
        }

        Ok(())
    }

    pub async fn history(config: &Config) -> anyhow::Result<()> {
        println!("Clipboard history requires a paired device's token; open the dashboard on a paired device to view it.");
        let _ = config;
        Ok(())
    }
}
