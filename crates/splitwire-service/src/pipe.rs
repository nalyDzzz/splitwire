//! Named pipe server: one JSON request and one JSON response per connection.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use splitwire_core::consts::{PIPE_NAME, PROTOCOL_VERSION};
use splitwire_core::ipc::{self, Request, Response, MAX_MESSAGE_BYTES};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::engine::Engine;

/// SYSTEM and Administrators get full control; interactive users (the person
/// at the keyboard, including over Remote Desktop) may read and write.
/// Network logons and services running as other accounts can't connect.
#[cfg(windows)]
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

const READ_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(windows)]
pub async fn serve(engine: Arc<Engine>) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    let security = crate::win::SecurityAttributes::from_sddl(PIPE_SDDL)?;
    let create = |first: bool| -> std::io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        options.first_pipe_instance(first).reject_remote_clients(true);
        // SAFETY: `security` outlives every pipe instance created here.
        unsafe { options.create_with_security_attributes_raw(PIPE_NAME, security.as_ptr()) }
    };

    let mut server = create(true).with_context(|| {
        format!("couldn't create {PIPE_NAME}; is another Splitwire service already running? (try `splitwire-service stop`)")
    })?;
    tracing::info!("Listening on {PIPE_NAME}");

    loop {
        server.connect().await.context("pipe connect failed")?;
        let client = server;
        server = create(false).context("couldn't create the next pipe instance")?;
        let engine = engine.clone();
        tokio::spawn(async move {
            if let Err(error) = handle(client, &engine).await {
                tracing::debug!("pipe client error: {error:#}");
            }
        });
    }
}

#[cfg(not(windows))]
pub async fn serve(_engine: Arc<Engine>) -> anyhow::Result<()> {
    anyhow::bail!("the Splitwire service only runs on Windows")
}

async fn handle<S>(stream: S, engine: &Engine) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader.take(MAX_MESSAGE_BYTES as u64));
    let mut line = String::new();
    tokio::time::timeout(READ_TIMEOUT, reader.read_line(&mut line))
        .await
        .context("timed out reading request")??;

    let response = match serde_json::from_str::<Request>(&line) {
        Ok(request) => dispatch(request, engine).await,
        Err(error) => Response::Error { message: format!("Bad request: {error}") },
    };
    writer.write_all(&ipc::encode(&response)).await?;
    writer.flush().await?;
    Ok(())
}

async fn dispatch(request: Request, engine: &Engine) -> Response {
    match request {
        Request::Hello => Response::Hello {
            protocol: PROTOCOL_VERSION,
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            engine_version: engine.engine_version().map(str::to_string),
        },
        Request::GetStatus => Response::Status { status: engine.status() },
        Request::Connect { tunnel, options } => match engine.connect(*tunnel, options).await {
            Ok(()) => Response::Ok,
            Err(message) => Response::Error { message },
        },
        Request::Disconnect => {
            engine.disconnect().await;
            Response::Ok
        }
        Request::GetConnections => Response::Connections { connections: engine.connections() },
        Request::GetLogs { after } => Response::Logs { entries: engine.logs(after) },
    }
}
