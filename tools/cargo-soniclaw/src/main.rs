use std::{
    error::Error,
    path::{Path, PathBuf},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("soniclaw: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn Error>> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("soniclaw") {
        args.remove(0);
    }
    if args.first().is_some_and(|a| {
        ["check", "doctor", "build-formats", "compat", "bundle"].contains(&a.as_str())
    }) {
        let action = args.remove(0);
        let mut framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        if let Some(index) = args.iter().position(|a| a == "--framework") {
            framework = PathBuf::from(args.get(index + 1).ok_or("missing --framework path")?);
            args.drain(index..=index + 1);
        }
        let framework = framework.canonicalize()?;
        if action == "doctor" {
            println!("Soniclaw workspace: {}\nPlatform: {} / {}\nNative audio and DAW validation must run on the target machine.",framework.display(),std::env::consts::OS,std::env::consts::ARCH);
            let status = std::process::Command::new("rustc")
                .arg("--version")
                .status()?;
            if !status.success() {
                return Err("rustc unavailable".into());
            }
            return Ok(());
        }
        if action == "check" {
            for flags in [
                vec!["fmt", "--all", "--check"],
                vec![
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--all-features",
                    "--",
                    "-D",
                    "warnings",
                ],
                vec!["test", "--workspace", "--all-features"],
            ] {
                if !std::process::Command::new("cargo")
                    .args(flags)
                    .current_dir(&framework)
                    .status()?
                    .success()
                {
                    return Err("workspace validation failed".into());
                }
            }
            return Ok(());
        }
        let script = match action.as_str() {
            "build-formats" => "build-formats.py",
            "compat" => "run-compat.py",
            _ => "bundle-clap.py",
        };
        let python = if cfg!(windows) { "python" } else { "python3" };
        if !std::process::Command::new(python)
            .arg(framework.join("tools").join(script))
            .args(args)
            .status()?
            .success()
        {
            return Err("Soniclaw tool failed".into());
        }
        return Ok(());
    }
    if args.is_empty() || args[0] == "--help" {
        println!("cargo soniclaw new <project-name> [--framework /path/to/soniclaw] [--kind app|clap|sampler]\n\nCreates a native app, CLAP effect or multisample instrument using this local Soniclaw checkout.\nCrates are not yet published to crates.io. Never overwrites a directory.\nAdditional commands: doctor, check, build-formats, compat, bundle (all accept --framework <path>).");
        return Ok(());
    }
    if args.len() < 2 || args[0] != "new" {
        return Err("expected: new <project-name> [--framework <path>]".into());
    }
    let name = &args[1];
    if !name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        || name.ends_with('-')
    {
        return Err("use a lowercase project name starting with a letter, containing letters, numbers or hyphens".into());
    }
    let mut framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut kind = "app";
    let mut index = 2;
    while index < args.len() {
        let value = args.get(index + 1).ok_or("missing option value")?;
        match args[index].as_str() {
            "--framework" => framework = PathBuf::from(value),
            "--kind" if value == "app" || value == "clap" || value == "sampler" => kind = value,
            _ => return Err("expected --framework <path> or --kind app|clap|sampler".into()),
        }
        index += 2;
    }
    let framework = framework.canonicalize()?;
    let facade = framework.join("crates/soniclaw");
    if !facade.join("Cargo.toml").is_file() {
        return Err("framework must point to a Soniclaw workspace".into());
    }
    let destination = Path::new(name);
    std::fs::create_dir(destination)?;
    std::fs::create_dir(destination.join("src"))?;
    // TOML basic-string escapes, including Windows path separators.
    let path = facade
        .to_str()
        .ok_or("framework path must be UTF-8")?
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    let manifest = if kind != "app" {
        let adapter = framework
            .join("crates/soniclaw-clap")
            .to_str()
            .ok_or("framework path must be UTF-8")?
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[workspace]

[lib]
crate-type = ["cdylib"]

[dependencies]
soniclaw = {{ path = "{path}" }}
soniclaw-clap = {{ path = "{adapter}" }}

[features]
default = ["editor"]
editor = ["soniclaw-clap/editor"]
"#
        )
    } else {
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[workspace]

[dependencies]
soniclaw = {{ path = "{path}", features = ["native", "ui"] }}
eframe = {{ version = "=0.36.1", default-features = false, features = ["glow", "x11", "wayland", "default_fonts"] }}
"#
        )
    };
    std::fs::write(destination.join("Cargo.toml"), manifest)?;
    if kind != "app" {
        std::fs::write(
            destination.join("src/lib.rs"),
            (if kind == "sampler" {
                include_str!("template-sampler.rs.txt")
            } else {
                include_str!("template-clap.rs.txt")
            })
            .replace("__PROJECT_NAME__", name),
        )?;
    } else {
        std::fs::write(
            destination.join("src/main.rs"),
            include_str!("template.rs.txt"),
        )?;
    }
    std::fs::write(destination.join(".gitignore"), "/target\n")?;
    let instructions = if kind != "app" {
        "Run `cargo build --release`, then use the framework's `tools/bundle-clap.py` to package the shared library. Change the provisional org.soniclaw product namespace before distribution. The default build includes an embedded editor. Use `--no-default-features` for a headless plug-in. See the framework format guide to build VST3/AUv2 wrappers. Sampler templates preload a small generated tone; replace its sample bank with your own assets."
    } else {
        "Run `cargo run --release`. Starts muted; raise Gain to hear it. Linux requires libasound2-dev and pkg-config; macOS requires Xcode command-line tools; Windows requires the Rust MSVC toolchain."
    };
    std::fs::write(destination.join("README.md"), format!("# {name}\n\n{instructions}\n\nThis project depends on a local Soniclaw checkout. Keep it available, or update the manifest paths.\n"))?;
    println!("Created {} ({kind})", destination.display());
    Ok(())
}
