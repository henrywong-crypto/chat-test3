use std::{env, fs, path::Path, process::Command};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let frontend_dir = Path::new(&manifest_dir).join("frontend");
    let src_dir = frontend_dir.join("src");
    let dist_dir = frontend_dir.join("dist");

    println!("cargo:rerun-if-changed={}", src_dir.join("styles.css").display());
    println!("cargo:rerun-if-changed={}", frontend_dir.join("package.json").display());

    fs::create_dir_all(&dist_dir).expect("failed to create frontend/dist");

    if !node_available() {
        return;
    }

    let node_modules = frontend_dir.join("node_modules");
    if !node_modules.exists() {
        run_command(Command::new("npm").arg("install").current_dir(&frontend_dir));
    }

    run_command(Command::new("npm").args(["run", "build"]).current_dir(&frontend_dir));
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_command(command: &mut Command) {
    let status = command
        .status()
        .unwrap_or_else(|e| panic!("failed to run {:?}: {e}", command.get_program()));
    if !status.success() {
        panic!("{:?} exited with {status}", command.get_program());
    }
}
