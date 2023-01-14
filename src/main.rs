use argh::FromArgs;
use tokio::net::TcpListener;
use tokio_socks::tcp::Socks5Stream;
use trust_dns_resolver::config::*;
use trust_dns_resolver::AsyncResolver;

#[derive(FromArgs)]
/// A simple Minecraft proxy
struct Args {
    /// the Minecraft server to connect to
    #[argh(option)]
    server: String,
    /// the proxy to use (proxy:port)
    #[argh(option)]
    proxy: String,
    /// proxy auth username
    #[argh(option)]
    username: Option<String>,
    /// proxy auth password
    #[argh(option)]
    password: Option<String>,
}

async fn run_server(
    mc_server: &str,
    proxy: &str,
    proxy_username: Option<&str>,
    proxy_password: Option<&str>,
) -> anyhow::Result<()> {
    let server = TcpListener::bind("0.0.0.0:1337").await?;
    println!(
        "Forwarding from {} to {} through proxy {} (auth: {})",
        server.local_addr()?,
        mc_server,
        proxy,
        proxy_username.is_some() || proxy_password.is_some()
    );
    loop {
        let (mut socket, _) = server.accept().await?;

        let target_server = match resolve_srv_record(&mc_server).await {
            Ok(srv) => format!(
                "{}:25565",
                if srv.ends_with('.') {
                    &srv[..srv.len() - 1]
                } else {
                    &srv
                }
            ),
            Err(e) => {
                println!("Error resolving SRV record: {}", e);
                continue;
            }
        };

        println!(
            "Forwarding client {} to {}...",
            socket.peer_addr()?,
            target_server
        );

        let socks5_stream = match (proxy_username, proxy_password) {
            (Some(username), Some(password)) => {
                Socks5Stream::connect_with_password(
                    proxy,
                    target_server.clone(),
                    username,
                    password,
                )
                .await
            }
            (Some(username), None) => {
                Socks5Stream::connect_with_password(proxy, target_server.clone(), username, "")
                    .await
            }
            (None, Some(password)) => {
                Socks5Stream::connect_with_password(proxy, target_server.clone(), "", password)
                    .await
            }
            (None, None) => Socks5Stream::connect(proxy, target_server.clone()).await,
        };

        let mut connection = match socks5_stream {
            Ok(c) => c,
            Err(e) => {
                println!("Error connecting to {}: {}", &target_server, e);
                continue;
            }
        };

        tokio::spawn(async move {
            tokio::io::copy_bidirectional(&mut socket, &mut connection)
                .await
                .unwrap();
        });
    }
}

async fn resolve_srv_record(domain: &str) -> anyhow::Result<String> {
    let config = ResolverConfig::default();
    let resolver = AsyncResolver::tokio(config, ResolverOpts::default())?;
    let srv = resolver
        .srv_lookup(format!("_minecraft._tcp.{}", domain))
        .await?;
    if let Some(srv) = srv.iter().next() {
        return Ok(srv.target().to_string());
    }
    let a = resolver.ipv4_lookup(domain).await?;
    if let Some(a) = a.iter().next() {
        return Ok(a.to_string());
    }
    anyhow::bail!("No A record found for {}", domain);
}

#[tokio::main]
async fn main() {
    let args: Args = argh::from_env();
    run_server(
        &args.server,
        &args.proxy,
        args.username.as_deref(),
        args.password.as_deref(),
    )
    .await
    .expect("Error running server");
}
