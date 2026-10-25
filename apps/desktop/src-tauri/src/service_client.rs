//! Talks to the Splitwire service over its named pipe.

use std::time::Duration;

use serde::Serialize;
use splitwire_core::consts::PIPE_NAME;
use splitwire_core::ipc::{self, EngineStatus, Request, Response, MAX_MESSAGE_BYTES};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// Starting a tunnel waits for sing-box to come up; everything else is quick.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("the Splitwire service isn't running")]
    NotRunning,
    #[error("the Splitwire service didn't respond in time")]
    Timeout,
    #[error("{0}")]
    Service(String),
    #[error("couldn't talk to the Splitwire service: {0}")]
    Io(#[from] std::io::Error),
    #[error("unexpected reply from the Splitwire service")]
    Protocol,
}

pub async fn request(request: Request) -> Result<Response, ClientError> {
    let timeout = if matches!(request, Request::Connect { .. }) { CONNECT_TIMEOUT } else { DEFAULT_TIMEOUT };
    match tokio::time::timeout(timeout, exchange(&request)).await {
        Ok(Ok(Response::Error { message })) => Err(ClientError::Service(message)),
        Ok(result) => result,
        Err(_) => Err(ClientError::Timeout),
    }
}

#[cfg(windows)]
async fn exchange(request: &Request) -> Result<Response, ClientError> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_PIPE_BUSY: i32 = 231;

    let pipe = loop {
        match ClientOptions::new().open(PIPE_NAME) {
            Ok(pipe) => break pipe,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => return Err(ClientError::NotRunning),
            Err(e) => return Err(e.into()),
        }
    };

    let (reader, mut writer) = tokio::io::split(pipe);
    writer.write_all(&ipc::encode(request)).await?;
    writer.flush().await?;
    let mut line = String::new();
    BufReader::new(reader.take(MAX_MESSAGE_BYTES as u64)).read_line(&mut line).await?;
    serde_json::from_str(&line).map_err(|_| ClientError::Protocol)
}

#[cfg(not(windows))]
async fn exchange(_request: &Request) -> Result<Response, ClientError> {
    Err(ClientError::NotRunning)
}

/// What the UI shows about the service. Mirrors `ServiceStatus` in types.ts.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServiceStatus {
    NotInstalled,
    Stopped,
    Unreachable { message: String },
    Running { service_version: String, engine_version: Option<String>, status: Box<EngineStatus> },
}

impl ServiceStatus {
    pub fn engine(&self) -> Option<&EngineStatus> {
        match self {
            ServiceStatus::Running { status, .. } => Some(status.as_ref()),
            _ => None,
        }
    }
}

/// Versions are cached between polls; pass `None` to fetch them.
pub async fn probe(versions: &mut Option<(String, Option<String>)>) -> ServiceStatus {
    if versions.is_none() {
        match request(Request::Hello).await {
            Ok(Response::Hello { service_version, engine_version, .. }) => {
                *versions = Some((service_version, engine_version));
            }
            Ok(_) => return ServiceStatus::Unreachable { message: ClientError::Protocol.to_string() },
            Err(ClientError::NotRunning) => return installed_state(),
            Err(e) => return ServiceStatus::Unreachable { message: e.to_string() },
        }
    }

    match request(Request::GetStatus).await {
        Ok(Response::Status { status }) => {
            let (service_version, engine_version) = versions.clone().unwrap_or_default();
            ServiceStatus::Running { service_version, engine_version, status: Box::new(status) }
        }
        Ok(_) => ServiceStatus::Unreachable { message: ClientError::Protocol.to_string() },
        Err(ClientError::NotRunning) => {
            *versions = None;
            installed_state()
        }
        Err(e) => {
            *versions = None;
            ServiceStatus::Unreachable { message: e.to_string() }
        }
    }
}

/// When the pipe isn't there: is the service installed at all?
#[cfg(windows)]
fn installed_state() -> ServiceStatus {
    use splitwire_core::consts::SERVICE_NAME;
    use windows_service::service::ServiceAccess;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let Ok(manager) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT) else {
        return ServiceStatus::Stopped;
    };
    match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        Ok(_) => ServiceStatus::Stopped,
        Err(_) => ServiceStatus::NotInstalled,
    }
}

#[cfg(not(windows))]
fn installed_state() -> ServiceStatus {
    ServiceStatus::NotInstalled
}
