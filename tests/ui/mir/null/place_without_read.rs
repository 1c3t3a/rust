//@ run-pass
//@ compile-flags: -C debug-assertions

fn main() {
    let ptr: *const u16 = std::ptr::null();
    unsafe {
        let _ = *ptr;
    }
}
