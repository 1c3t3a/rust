//@ run-pass
//@ compile-flags: -C debug-assertions

fn main() {
    let ptr: *mut () = std::ptr::null_mut();
    unsafe {
        *(ptr) = ();
    }
}
