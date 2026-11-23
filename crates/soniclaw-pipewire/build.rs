fn main() {
    println!("cargo:rerun-if-changed=src/bridge.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let lib = pkg_config::Config::new()
        .atleast_version("0.3.65")
        .probe("libpipewire-0.3")
        .expect("install PipeWire/SPA development headers and pkg-config");
    let mut build = cc::Build::new();
    build
        .file("src/bridge.c")
        .define("_GNU_SOURCE", None)
        .flag_if_supported("-std=gnu11")
        .flag_if_supported("-Werror");
    for path in lib.include_paths {
        build.include(path);
    }
    build.compile("soniclaw_pipewire_bridge");
}
