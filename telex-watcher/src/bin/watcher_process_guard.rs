fn main() {
    if let Err(error) = telex_watcher::process::guard_main() {
        eprintln!("watcher process guard: {error}");
        std::process::exit(1);
    }
}
