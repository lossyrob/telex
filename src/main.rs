use telex::{cli, install};

fn main() {
    match install::maybe_dispatch_launcher() {
        Ok(Some(code)) => std::process::exit(code),
        Ok(None) => {}
        Err(e) => {
            eprintln!("telex launcher: {e:#}");
            std::process::exit(1);
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("create tokio runtime");
    let code = runtime.block_on(cli::run());
    telex::profiles::drain_password_commands_before_exit();
    std::process::exit(code);
}
