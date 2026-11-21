//! Compatibility entry point for the upstream host userspace binary.
use std::os::unix::process::CommandExt;
fn main() {
    let path = std::env::current_exe().expect("executable path")
        .parent().expect("executable directory").join("host/bin/nash");
    let error = std::process::Command::new(&path)
        .args(std::env::args_os().skip(1))
        .env("PATH", format!("{}:{}", path.parent().unwrap().display(),
            std::env::var("PATH").unwrap_or_default()))
        .exec();
    eprintln!("{}: {error}; run bash scripts/build-upstream-userspace.sh", path.display());
    std::process::exit(127);
}
