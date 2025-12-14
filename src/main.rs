mod cache_affinity;
mod provider;
mod router;
mod server;
mod settings;

use anyhow::Result;
use cache_affinity::CacheAffinityManager;
use local_ip_address::{list_afinet_netifas, local_ip};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use router::Router;
use std::env;
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:18100";
const CACHE_TTL: u64 = 300; // 5 minutes

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args: Vec<String> = env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("start") => start_daemon().await,
        Some("stop") => stop_daemon(),
        Some("status") => show_status(),
        Some("help") | Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        _ => {
            println!("Usage: cc-proxy [start|stop|status|help]");
            println!("Run 'cc-proxy help' for more information");
            Ok(())
        }
    }
}

async fn start_daemon() -> Result<()> {
    // Check if already running
    if is_running() {
        println!("❌ cc-proxy is already running");
        println!("   Run 'cc-proxy stop' to stop it first");
        process::exit(1);
    }

    println!("🚀 Starting cc-proxy...");
    println!();

    // Write PID file
    let pid = process::id();
    write_pid_file(pid)?;

    let advertise_addr = detect_advertise_addr(DEFAULT_BIND_ADDR);

    // Configure CLI tools
    println!("⚙️  Configuring CLI tools...");
    if let Err(e) = settings::configure_all(&advertise_addr) {
        tracing::warn!("Failed to configure CLI tools: {}", e);
        println!("⚠️  Warning: Failed to configure CLI tools automatically");
        println!("   You may need to configure Claude Code and Codex manually");
    }
    println!();

    // Initialize cache affinity manager
    let affinity_manager = Arc::new(CacheAffinityManager::new(CACHE_TTL));

    // Start cleanup task
    CacheAffinityManager::start_cleanup_task(affinity_manager.clone());

    // Keep provider responses compressed so headers stay consistent end-to-end.
    let http_client = reqwest::Client::builder()
        .no_gzip()
        .no_deflate()
        .no_brotli()
        .build()?;

    // Initialize router
    let router = Arc::new(Router::new(affinity_manager.clone(), http_client)?);

    // Start config file watcher
    start_config_watcher(router.clone())?;

    // Start server
    println!("✨ cc-proxy is running!");
    println!("   Listening on:   http://{}", DEFAULT_BIND_ADDR);
    println!("   Share this URL: http://{}", advertise_addr);
    println!("   Claude Code: POST /v1/messages");
    println!("   Codex:       POST /responses");
    println!();
    println!("💡 Tip: Edit ~/.cc-proxy/provider.json to configure providers");
    println!();

    // Run server (blocks until shutdown)
    server::run_server(router, DEFAULT_BIND_ADDR).await?;

    // Cleanup on shutdown
    remove_pid_file()?;

    Ok(())
}

fn start_config_watcher(router: Arc<Router>) -> Result<()> {
    // Get config file path
    let config_path = provider::get_config_path()?;

    tracing::info!("Starting config file watcher");
    tracing::debug!("Watching: {:?}", config_path);

    // Create async channel for file events
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    // Spawn watcher in a blocking thread (notify requires blocking context)
    std::thread::spawn(move || {
        let tx_clone = tx.clone();
        let mut watcher = match RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx_clone.send(event);
                }
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("Failed to create file watcher: {}", e);
                return;
            }
        };

        // Watch config file
        if let Err(e) = watcher.watch(&config_path, RecursiveMode::NonRecursive) {
            tracing::warn!("Failed to watch config: {}", e);
        }

        // Keep watcher alive
        loop {
            std::thread::park();
        }
    });

    // Spawn async task to handle file events
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            // Only reload on modify/create events
            if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                continue;
            }

            tracing::info!("Config file changed: {:?}", event.paths);

            // Reload providers with a small delay to avoid partial writes
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            if let Err(e) = router.reload_providers().await {
                tracing::error!("Failed to reload providers: {}", e);
            }
        }
    });

    Ok(())
}

fn stop_daemon() -> Result<()> {
    if !is_running() {
        println!("cc-proxy is not running");
        return Ok(());
    }

    let pid = read_pid_file()?;

    println!("Stopping cc-proxy (PID: {})...", pid);

    // Send SIGTERM
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("kill")
            .arg(pid.to_string())
            .output()
            .expect("Failed to send kill signal");
    }

    remove_pid_file()?;
    println!("✓ cc-proxy stopped");

    Ok(())
}

fn show_status() -> Result<()> {
    if !is_running() {
        println!("Status: ❌ Not running");
        return Ok(());
    }

    let pid = read_pid_file()?;
    println!("Status: ✅ Running");
    println!("PID:    {}", pid);
    println!("Bind:   http://{}", DEFAULT_BIND_ADDR);
    println!(
        "Share:  http://{}",
        detect_advertise_addr(DEFAULT_BIND_ADDR)
    );

    Ok(())
}

fn print_help() {
    println!("cc-proxy - HTTP Proxy for Claude Code & Codex");
    println!();
    println!("USAGE:");
    println!("    cc-proxy [COMMAND]");
    println!();
    println!("COMMANDS:");
    println!("    start     Start the proxy daemon");
    println!("    stop      Stop the proxy daemon");
    println!("    status    Show proxy status");
    println!("    help      Show this help message");
    println!();
    println!("DESCRIPTION:");
    println!("    cc-proxy is a smart HTTP proxy that routes Claude Code and Codex");
    println!("    requests to multiple providers with automatic failover and cache");
    println!("    affinity for maximum cost savings.");
    println!();
    println!("FEATURES:");
    println!("    • Model-aware routing (supports exact and wildcard matching)");
    println!("    • Cache affinity (maintains provider for 5min for cache hits)");
    println!("    • Automatic failover (tries multiple providers)");
    println!("    • Auto-configuration (sets up Claude Code & Codex)");
    println!();
    println!("CONFIGURATION:");
    println!("    ~/.cc-proxy/provider.json");
    println!();
    println!("EXAMPLES:");
    println!("    # Start the proxy");
    println!("    cc-proxy start");
    println!();
    println!("    # Check if running");
    println!("    cc-proxy status");
    println!();
    println!("    # Stop the proxy");
    println!("    cc-proxy stop");
    println!();
    println!("For more information: https://github.com/yourusername/cc-proxy");
}

fn detect_advertise_addr(bind_addr: &str) -> String {
    let socket = bind_addr.parse::<SocketAddr>().ok();
    let port = socket.map(|sock| sock.port()).unwrap_or(18100);

    if let Some(socket) = socket {
        let ip = socket.ip();
        if !ip.is_loopback() && !ip.is_unspecified() {
            return format!("{}:{}", ip, port);
        }
    }

    if let Some(ip) = pick_local_ip() {
        let addr = format!("{}:{}", ip, port);
        tracing::info!("Detected LAN address for CLI config: {}", addr);
        return addr;
    }

    tracing::warn!(
        "Falling back to {} for CLI configuration; could not detect LAN IP",
        DEFAULT_BIND_ADDR
    );
    DEFAULT_BIND_ADDR.to_string()
}

fn pick_local_ip() -> Option<IpAddr> {
    if let Ok(netifs) = list_afinet_netifas() {
        let mut ipv4_candidate = None;
        let mut ipv6_candidate = None;

        for (iface, ip) in netifs {
            if is_virtual_iface(&iface) || !is_usable_ip(&ip) {
                continue;
            }

            match ip {
                IpAddr::V4(_) => {
                    ipv4_candidate = Some(ip);
                    break;
                }
                IpAddr::V6(_) => {
                    ipv6_candidate = ipv6_candidate.or(Some(ip));
                }
            }
        }

        if ipv4_candidate.is_some() {
            return ipv4_candidate;
        }
        if ipv6_candidate.is_some() {
            return ipv6_candidate;
        }
    }

    if let Ok(ip) = local_ip() {
        if is_usable_ip(&ip) {
            return Some(ip);
        }
    }

    None
}

fn is_virtual_iface(iface: &str) -> bool {
    let name = iface.to_ascii_lowercase();
    matches!(name.as_str(), "lo" | "localhost" | "loopback")
        || name.starts_with("docker")
        || name.starts_with("br-")
        || name.starts_with("veth")
        || name.starts_with("virbr")
        || name.starts_with("vmnet")
        || name.starts_with("tailscale")
        || name.starts_with("wg")
        || name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("zt")
}

fn is_ipv6_unicast_link_local(v6: &std::net::Ipv6Addr) -> bool {
    // fe80::/10
    let seg0 = v6.segments()[0];
    (seg0 & 0xffc0) == 0xfe80
}

fn is_usable_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return false;
            }
            let octets = v4.octets();
            if octets[0] == 169 && octets[1] == 254 {
                return false; // IPv4 link-local
            }
            true
        }
        IpAddr::V6(v6) => {
            // Treat IPv6 Unique Local Addresses (ULA, fc00::/7) as usable, analogous to
            // private IPv4 (e.g., 192.168.0.0/16). Still reject loopback/unspecified/link-local.
            if v6.is_loopback() || v6.is_unspecified() {
                return false;
            }
            !is_ipv6_unicast_link_local(v6)
        }
    }
}

// Helper functions for PID file management
fn get_pid_file_path() -> Result<PathBuf> {
    let home = env::var("HOME")?;
    let pid_dir = PathBuf::from(home).join(".cc-proxy");
    fs::create_dir_all(&pid_dir)?;
    Ok(pid_dir.join("cc-proxy.pid"))
}

fn write_pid_file(pid: u32) -> Result<()> {
    let path = get_pid_file_path()?;
    fs::write(path, pid.to_string())?;
    Ok(())
}

fn read_pid_file() -> Result<u32> {
    let path = get_pid_file_path()?;
    let content = fs::read_to_string(path)?;
    Ok(content.trim().parse()?)
}

fn remove_pid_file() -> Result<()> {
    let path = get_pid_file_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn is_running() -> bool {
    let Ok(pid_path) = get_pid_file_path() else {
        return false;
    };

    if !pid_path.exists() {
        return false;
    }

    let Ok(pid) = read_pid_file() else {
        return false;
    };

    // Check if process is actually running
    #[cfg(unix)]
    {
        use std::process::Command;
        let output = Command::new("kill").arg("-0").arg(pid.to_string()).output();

        matches!(output, Ok(o) if o.status.success())
    }

    #[cfg(not(unix))]
    {
        true // Assume running on non-Unix systems
    }
}
