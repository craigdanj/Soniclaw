fn main() {
    // SAFETY: dedicated child executable, module selection is explicit in the parent API.
    if let Err(error) = unsafe { soniclaw_host::isolation::helper_main() } {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
