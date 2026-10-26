fn main() {
    // Leave time for the capture debugger to attach.
    std::thread::sleep(std::time::Duration::from_secs(8));
    // Rust's abort on Windows is __fastfail, which exits 0xc0000409 like #203.
    std::process::abort();
}
