//@ check-pass

#![warn(returning_pointers_to_local_variables)]

fn foo() -> *const u32 {
    let empty = 42u32;
    return &empty as *const _;
    //~^ WARN returning a pointer to stack memory associated with a local variable
}

fn bar() -> *const u32 {
    let empty = 42u32;
    &empty as *const _
    //~^ WARN returning a pointer to stack memory associated with a local variable
}

fn baz() -> *const u32 {
    let empty = 42u32;
    return &empty;
    //~^ WARN returning a pointer to stack memory associated with a local variable
}

fn faa() -> *const u32 {
    let empty = 42u32;
    &empty
    //~^ WARN returning a pointer to stack memory associated with a local variable
}

fn main() {}
