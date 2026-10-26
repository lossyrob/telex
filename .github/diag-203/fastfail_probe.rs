fn main() {
    // Rust's abort on Windows is __fastfail, which exits 0xc0000409 like #203.
    std::process::abort();
}
