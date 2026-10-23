use clap::Parser;
use std::process::ExitCode;
use telex_watcher::cli::{Cli, InputError};
use telex_watcher::management::{Command, Response};
use telex_watcher::presentation::{transport_error_message, Envelope};
use telex_watcher::{transport, Runtime, RuntimeConfig};

#[tokio::main]
async fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args_os().collect();
    let args = match Cli::try_parse_from(&arguments) {
        Ok(args) => args,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                let _ = error.print();
                return ExitCode::SUCCESS;
            }
            if arguments.iter().any(|argument| argument == "--json") {
                print_envelope(
                    &Envelope::error(
                        "parse",
                        "invalid-arguments",
                        "invalid or missing command arguments; use --help",
                    ),
                    true,
                );
            } else {
                let _ = error.print();
            }
            return ExitCode::from(2);
        }
    };
    let Some(registry) = args.registry.as_ref() else {
        print_envelope(
            &Envelope::error(
                args.command_name(),
                "invalid-arguments",
                "--registry or TELEX_WATCHER_REGISTRY is required",
            ),
            args.json,
        );
        return ExitCode::from(2);
    };
    let command_name = args.command_name();
    let envelope = if args.is_runtime() {
        match run_host(&args).await {
            Ok(()) => Envelope::from_response(
                command_name,
                &Response::Stopping,
                None,
                chrono::Utc::now().timestamp_millis(),
            ),
            Err(error) => Envelope::error(command_name, error.code, error.message),
        }
    } else {
        match args.management_command() {
            Err(error) => Envelope::error(command_name, error.code, error.message),
            Ok(command) => {
                match transport::execute(registry, command, args.filters().cloned()).await {
                    Ok(mut envelope) => {
                        envelope.command = command_name.into();
                        envelope
                    }
                    Err(error) => Envelope::error(
                        command_name,
                        error.code(),
                        transport_error_message(error.code()),
                    ),
                }
            }
        }
    };
    let exit = if envelope.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    };
    print_envelope(&envelope, args.json);
    exit
}

fn print_envelope(envelope: &Envelope, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(envelope).expect("safe output serialization")
        );
    } else {
        println!("{}", envelope.human());
    }
}

async fn run_host(args: &Cli) -> Result<(), InputError> {
    let install_root = args.install_root.clone().ok_or(InputError {
        code: "install-root-required",
        message: "runtime startup requires --install-root or TELEX_INSTALL_ROOT",
    })?;
    let mut config = RuntimeConfig::installed(
        args.registry
            .clone()
            .expect("registry checked before runtime dispatch"),
        install_root,
    );
    config.concurrency = usize::from(args.concurrency);
    host(config).await.map_err(|error| {
        if error.chain().any(|cause| cause.to_string() == "registry-already-owned") {
            InputError {
                code: "registry-already-owned",
                message: "another runtime owns this registry; manage the existing owner instead",
            }
        } else if let Some(error) = error.downcast_ref::<transport::TransportError>() {
            InputError {
                code: error.code(),
                message: transport_error_message(error.code()),
            }
        } else {
            InputError {
                code: "runtime-failed",
                message: "runtime startup or execution failed; verify host configuration and retained runtime diagnostics",
            }
        }
    })
}

async fn host(config: RuntimeConfig) -> anyhow::Result<()> {
    let registry = config.registry.clone();
    let (runtime, management) = Runtime::open(config)?;
    let ownership = runtime.retain_registry_ownership();
    // Binding must follow ownership acquisition, never create an offline management owner.
    let server = transport::Server::bind(&registry, management.clone())?;
    let mut server = tokio::spawn(server.run());
    let mut owner = tokio::spawn(runtime.run());
    let mut server_finished = false;
    let mut transport_shutdown = true;
    let result = tokio::select! {
        result = &mut owner => result.map_err(anyhow::Error::from).and_then(|result| result),
        result = &mut server => {
            server_finished = true;
            match result {
                Ok(Ok(())) => owner.await.map_err(anyhow::Error::from).and_then(|result| result),
                result => {
                    let _ = management.execute(Command::Shutdown).await;
                    let _ = owner.await;
                    match result {
                        Ok(Err(error)) => Err(error),
                        Err(error) => Err(error.into()),
                        Ok(Ok(())) => unreachable!(),
                    }
                }
            }
        }
        signal = shutdown_signal() => {
            transport_shutdown = false;
            let shutdown = management.execute(Command::Shutdown).await;
            let result = owner.await.map_err(anyhow::Error::from).and_then(|result| result);
            signal.and(shutdown.map(|_| ())).and(result)
        }
    };
    if !server_finished && result.is_ok() && transport_shutdown {
        // A transport Shutdown must finish its Stopping response before listener cleanup.
        server_finished = tokio::time::timeout(
            transport::WRITE_TIMEOUT + std::time::Duration::from_secs(1),
            &mut server,
        )
        .await
        .is_ok();
    }
    if !server_finished {
        server.abort();
        let _ = server.await;
    }
    drop(ownership);
    result
}

async fn shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
