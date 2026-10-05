fn main() {
    // Windows reserves only 1 MiB for the main thread by default. Owned codec
    // parsing and the media CLI require the same stack headroom as Rust tests.
    // Keep GUI execution on the main thread and reserve a bounded 8 MiB stack.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bin=fvid=/STACK:8388608");
    }
}
