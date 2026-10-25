//! Windows service plumbing: the SCM entry point plus install/uninstall.

use std::ffi::OsString;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use splitwire_core::consts::{SERVICE_DESCRIPTION, SERVICE_DISPLAY_NAME, SERVICE_NAME};
use tokio::sync::watch;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept, ServiceErrorControl,
    ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState,
    ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

define_windows_service!(ffi_service_main, service_main);

/// Entry point when started by the Service Control Manager (`service` arg).
pub fn dispatch() -> anyhow::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("not started by the Service Control Manager; use `splitwire-service run` instead")
}

fn service_main(_args: Vec<OsString>) {
    let _guard = crate::logs::init_service_tracing();
    if let Err(error) = run_service() {
        tracing::error!("service failed: {error:#}");
    }
}

fn run_service() -> anyhow::Result<()> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let status_handle = service_control_handler::register(SERVICE_NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = shutdown_tx.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;

    let report = |state: ServiceState, exit_code: u32| {
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: state,
            controls_accepted: if state == ServiceState::Running {
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: ServiceExitCode::Win32(exit_code),
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        });
    };

    crate::win::ensure_console();
    report(ServiceState::Running, 0);
    let result = tokio::runtime::Runtime::new()
        .context("couldn't start the async runtime")
        .and_then(|runtime| runtime.block_on(crate::serve(shutdown_rx)));
    report(ServiceState::Stopped, if result.is_ok() { 0 } else { 1 });
    result
}

pub fn install() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .context("can't open the Service Control Manager (run this as administrator)")?;
    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: SERVICE_DISPLAY_NAME.into(),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?,
        launch_arguments: vec!["service".into()],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG;

    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => {
            stop_and_wait(&service)?;
            service.change_config(&info)?;
            println!("Updated existing service");
            service
        }
        Err(_) => {
            let service = manager.create_service(&info, access)?;
            println!("Installed {SERVICE_DISPLAY_NAME}");
            service
        }
    };
    service.set_description(SERVICE_DESCRIPTION)?;
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(2) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(10) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(60) },
        ]),
    })?;
    service.start::<&str>(&[]).context("installed, but the service didn't start")?;
    println!("Service started");
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("can't open the Service Control Manager (run this as administrator)")?;
    let service = match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
        Ok(service) => service,
        Err(_) => {
            println!("Service isn't installed");
            return Ok(());
        }
    };
    stop_and_wait(&service)?;
    service.delete()?;
    println!("Removed {SERVICE_DISPLAY_NAME}");
    Ok(())
}

pub fn start() -> anyhow::Result<()> {
    let service = open(ServiceAccess::START | ServiceAccess::QUERY_STATUS)?;
    if service.query_status()?.current_state != ServiceState::Running {
        service.start::<&str>(&[])?;
    }
    println!("Service started");
    Ok(())
}

pub fn stop() -> anyhow::Result<()> {
    let service = open(ServiceAccess::STOP | ServiceAccess::QUERY_STATUS)?;
    stop_and_wait(&service)?;
    println!("Service stopped");
    Ok(())
}

fn open(access: ServiceAccess) -> anyhow::Result<windows_service::service::Service> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("can't open the Service Control Manager")?;
    manager
        .open_service(SERVICE_NAME, access)
        .context("the Splitwire service isn't installed, or you need to run this as administrator")
}

fn stop_and_wait(service: &windows_service::service::Service) -> anyhow::Result<()> {
    if service.query_status()?.current_state == ServiceState::Stopped {
        return Ok(());
    }
    let _ = service.stop();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if service.query_status()?.current_state == ServiceState::Stopped {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    bail!("timed out waiting for the service to stop")
}
