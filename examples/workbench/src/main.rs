#[cfg(feature = "gui")]
mod gui;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if let Some(index) = args.iter().position(|x| x == "--render") {
        let path = args
            .get(index + 1)
            .ok_or("--render needs a new output directory")?;
        return soniclaw_workbench::demonstration(std::path::Path::new(path));
    }
    #[cfg(feature = "gui")]
    return gui::run(&args);
    #[cfg(not(feature = "gui"))]
    Err("use --render OUTPUT, or enable the gui feature".into())
}
