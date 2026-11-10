//! Prepare a raw bFS volume for bOS /users.
use bfs::{create_file, mkdir_p, mkfs, MkfsOptions};
use std::{fs::OpenOptions, io::Write, path::PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("mkusers: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("usage: mkusers <image-path>")?);
    if args.next().is_some() {
        return Err("usage: mkusers <image-path>".into());
    }
    let mut image = Vec::new();
    mkfs(
        &mut image,
        &MkfsOptions {
            label: "bOS-users".into(),
            size_mb: 16,
        },
    )?;
    for name in ["downloads", "documents", "pictures", "music", "videos"] {
        mkdir_p(&mut image, &format!("/default/{name}"))?;
    }
    create_file(&mut image, "/default/documents/notes.txt", b"")?;
    create_file(
        &mut image,
        "/default/.profile",
        include_bytes!("../../../../users/default/.profile"),
    )?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&image)?;
    file.sync_all()?;
    Ok(())
}
