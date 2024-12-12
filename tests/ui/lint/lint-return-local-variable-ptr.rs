//@ check-pass

#![warn(return_local_variable_ptr)]

fn foo() -> *const u32 {
    let empty = 42u32;
    return &empty as *const _;
}

fn bar() -> *const u32 {
    let empty = 42u32;
    &empty as *const _
}

fn baz() -> *const u32 {
    let empty = 42u32;
    return &empty;
}

fn faa() -> *const u32 {
    let empty = 42u32;
    &empty
}

fn main() {}
