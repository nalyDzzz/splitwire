//! The privileged half of Splitwire.
//!
//! Runs as a Windows service (LocalSystem) so the desktop app never needs
//! admin rights. It listens on a named pipe, and on request starts sing-box
//! with a config it generates itself from the tunnel it was sent.
//!
//! ```text
//! splitwire-service run         run in this console (needs an elevated terminal)
//! splitwire-service install     install and start the Windows service
//! splitwire-service uninstall   stop and remove the Windows service
//! splitwire-service start|stop  start or stop the installed service
//! ```

mod clash;
#[cfg(test)]
mod e2e_tests;
mod engine;
mod logs;
mod paths;
mod pipe;
#[cfg(windows)]
mod win;
#[cfg(windows)]
mod winsvc;

use std::process::ExitCode;
use std::sync::Arc;

use tokio::sync::watch;

use crate::engine::Engine;

const USAGE: &str = "\
Splitwire tunnel service

Usage: splitwire-service <command>

Commands:
  run         Run in this console (use an elevated terminal)
  install     Install and start the Windows service
  uninstall   Stop and remove the Windows service
  start       Start the installed service
  stop        Stop the installed service

Environment:
  SPLITWIRE_SING_BOX   Path to sing-box.exe (default: next to this program)
  RUST_LOG             Log filter, e.g. debug";

fn main() -> ExitCode {
    let command = std::env::args().nth(1).unwrap_or_default();
    let result = match command.as_str() {
        "run" => run_console(),
        #[cfg(windows)]
        "service" => winsvc::dispatch(),
        #[cfg(windows)]
        "install" => winsvc::install(),
        #[cfg(windows)]
        "uninstall" => winsvc::uninstall(),
        #[cfg(windows)]
        "start" => winsvc::start(),
        #[cfg(windows)]
        "stop" => winsvc::stop(),
        "" | "help" | "-h" | "--help" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("Unknown command \"{other}\"\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_console() -> anyhow::Result<()> {
    logs::init_console_tracing();
    #[cfg(windows)]
    if !win::is_elevated() {
        tracing::warn!("Not running as administrator: the pipe will work, but tunnels will fail to start");
    }

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Ctrl+C received, shutting down");
            let _ = shutdown_tx.send(true);
        });
        serve(shutdown_rx).await
    })
}

/// Runs the pipe server until `shutdown` flips to true, then stops any tunnel.
pub async fn serve(mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
    let engine = Arc::new(Engine::new(paths::sing_box_exe()?));
    tracing::info!(
        sing_box = %engine.sing_box_path().display(),
        version = engine.engine_version().unwrap_or("unknown"),
        "Splitwire service {} starting",
        env!("CARGO_PKG_VERSION"),
    );

    let result = tokio::select! {
        result = pipe::serve(engine.clone()) => result,
        _ = shutdown.wait_for(|stop| *stop) => Ok(()),
    };

    engine.disconnect().await;
    tracing::info!("Service stopped");
    result
}
