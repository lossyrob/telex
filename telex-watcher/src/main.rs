fn main() {
    if let Err(error) = telex_watcher::run() {
        eprintln!("telex-watcher: {error:#}");
        std::process::exit(1);
    }
}
