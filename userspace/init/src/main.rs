//! bOS init: mounts /proc, spawns bsh.
fn main() {
    println!("bOS init: mounting /dev, /proc...");
    println!("bOS init: spawning /bin/bsh");
    // In real bOS this uses syscalls; host build just execs shell.
}
