#[link(name = "kernel32")]
extern "system" {
    fn RaiseException(code: u32, flags: u32, count: u32, arguments: *const usize);
}

fn main() {
    // Only the disposable hosted capture preflight invokes this executable.
    unsafe {
        RaiseException(0xc000_0005, 1, 0, std::ptr::null());
    }
    panic!("the noncontinuable native exception unexpectedly returned");
}
