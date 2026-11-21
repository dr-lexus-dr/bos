//! Host init entry point. Native bOS PID 1 lives in native-demo/native-init.
use std::os::unix::process::CommandExt;
fn main() {
    let shell = std::env::current_exe().expect("executable path")
        .parent().expect("executable directory").join("bsh");
    let error = std::process::Command::new(&shell)
        .arg("--login")
        .args(std::env::args_os().skip(1))
        .exec();
    eprintln!("binit: {}: {error}", shell.display());
    std::process::exit(127);
}
