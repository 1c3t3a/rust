//@ run-pass
//@ compile-flags: -C debug-assertions

fn main() {
    let ptr: *const () = std::ptr::null();
    let _ptr = unsafe { *ptr };
}
