use super::Server;
use rmcp::{
    ServiceExt,
    transport::{
        stdio,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use std::{net::SocketAddr, sync::Arc};

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    const HELP: &str = "fvid mcp --root DIRECTORY [--stdio | --http 127.0.0.1:PORT] [--jobs 1..32]\nHTTP requires FVID_MCP_TOKEN (at least 16 characters). No uploads or public listener; paths are server-local.";
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    let mut root = None;
    let mut jobs = 1;
    let mut http = None;
    let mut stdio_mode = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--jobs" => {
                i += 1;
                jobs = args.get(i).ok_or("missing jobs")?.parse::<usize>()?;
            }
            "--root" => {
                i += 1;
                root = Some(args.get(i).ok_or("missing root")?);
            }
            "--http" => {
                i += 1;
                http = Some(
                    args.get(i)
                        .ok_or("missing address")?
                        .parse::<SocketAddr>()?,
                );
            }
            "--stdio" => stdio_mode = true,
            _ => return Err(HELP.into()),
        }
        i += 1;
    }
    if stdio_mode && http.is_some() {
        return Err("select one MCP transport".into());
    }
    let server = Server::with_jobs(root.ok_or("--root is required")?, jobs)?;
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(jobs + 4)
        .enable_all()
        .build()?
        .block_on(async move {
            if let Some(address) = http {
                if !address.ip().is_loopback() {
                    return Err("HTTP listener must use loopback".into());
                }
                let token = std::env::var("FVID_MCP_TOKEN")
                    .map_err(|_| "FVID_MCP_TOKEN required for HTTP")?;
                if token.len() < 16 || !token.bytes().all(|b| b.is_ascii_graphic()) {
                    return Err(
                        "HTTP token must contain at least 16 visible ASCII characters".into(),
                    );
                }
                let expected = Arc::new(format!("Bearer {token}"));
                let listener = tokio::net::TcpListener::bind(address).await?;
                let actual = listener.local_addr()?;
                let mut config = StreamableHttpServerConfig::default();
                config.legacy_session_mode = false;
                config.json_response = true;
                config.max_request_body_bytes = 1024 * 1024;
                config.allowed_origins = vec![
                    format!("http://{actual}"),
                    format!("http://localhost:{}", actual.port()),
                ];
                config.allowed_hosts =
                    vec![actual.to_string(), format!("localhost:{}", actual.port())];
                let service = StreamableHttpService::new(
                    move || Ok(server.clone()),
                    Arc::new(LocalSessionManager::default()),
                    config,
                );
                let app = axum::Router::new().nest_service("/mcp", service).layer(
                    axum::middleware::from_fn(
                        move |request: axum::extract::Request, next: axum::middleware::Next| {
                            let expected = expected.clone();
                            async move {
                                if request
                                    .headers()
                                    .get(axum::http::header::AUTHORIZATION)
                                    .and_then(|v| v.to_str().ok())
                                    != Some(expected.as_str())
                                {
                                    return axum::http::StatusCode::UNAUTHORIZED.into_response();
                                }
                                next.run(request).await
                            }
                        },
                    ),
                );
                use axum::response::IntoResponse;
                eprintln!("Fvid MCP listening on http://{actual}/mcp");
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = tokio::signal::ctrl_c().await;
                    })
                    .await?;
            } else {
                let running = server.serve(stdio()).await?;
                running.waiting().await?;
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        })
}
