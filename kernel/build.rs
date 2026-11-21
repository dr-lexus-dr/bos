use std::process::Command;
fn main() {
    if std::env::var("TARGET").as_deref() == Ok("x86_64-unknown-none") {
        ring_elf_assembly();
        build_i386_examples();
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/heads");
    let revision = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|r| r.status.success())
        .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .ok()
        .is_some_and(|r| !r.stdout.is_empty());
    println!(
        "cargo:rustc-env=BOS_SOURCE_REVISION={}{}",
        revision,
        if dirty { "-dirty" } else { "" }
    );
}

// ring builds its C sources on bare metal. Its list of ELF assembly targets
// omits target_os=none. Use the pre-generated ELF sources from the very same
// locked crate, including its version-specific symbol prefix.
fn ring_elf_assembly() {
    // Limit metadata to ring's dependency graph. A fresh single-package
    // firmware build need not have every host application cached offline.
    let probe = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("ring-source");
    std::fs::create_dir_all(&probe).unwrap();
    std::fs::write(
        probe.join("Cargo.toml"),
        concat!(
            "[package]\nname='bos-ring-source'\nversion='0.0.0'\nedition='2021'\n",
            "[workspace]\n[lib]\npath='empty.rs'\n[dependencies]\nring='0.17'\n"
        ),
    )
    .unwrap();
    std::fs::write(probe.join("empty.rs"), "").unwrap();
    std::fs::copy("../Cargo.lock", probe.join("Cargo.lock")).unwrap();
    let output = Command::new(std::env::var_os("CARGO").unwrap())
        .args([
            "metadata",
            "--offline",
            "--format-version",
            "1",
            "--filter-platform",
            "x86_64-unknown-none",
            "--manifest-path",
        ])
        .arg(probe.join("Cargo.toml"))
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "ring")
        .expect("locked ring package");
    let root = std::path::Path::new(package["manifest_path"].as_str().unwrap())
        .parent()
        .unwrap();
    let generated = root.join("pregenerated");
    let mut build = cc::Build::new();
    build.include(root.join("include")).include(&generated);
    let mut sources: Vec<_> = std::fs::read_dir(&generated)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.contains("x86_64") && name.ends_with("-elf.S")
        })
        .collect();
    sources.sort();
    build.files(sources).compile("bos_ring_elf");
    println!("cargo:rerun-if-changed=../Cargo.lock");
}

fn build_i386_examples() {
    let source=std::path::Path::new("../userspace/compat32");
    let out=std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=../userspace/compat32");
    for name in ["hello","primes","guess","counter","wc","spin","crash"] {
        let status=Command::new("clang").args(["--target=i386-unknown-none","-m32","-mno-sse","-Os","-ffreestanding","-fno-pic","-fno-stack-protector","-fno-unwind-tables","-fno-asynchronous-unwind-tables","-c"]).arg(source.join(format!("{name}.c"))).arg("-o").arg(out.join(format!("{name}.o"))).status().expect("clang for i386");
        assert!(status.success(),"compile {name}");
    }
    assert!(Command::new("as").arg("--32").arg(source.join("start.S")).arg("-o").arg(out.join("start32.o")).status().unwrap().success());
    for name in ["hello","primes","guess","counter","wc","spin","crash"]{
        assert!(Command::new("ld").args(["-m","elf_i386","-s","-T"]).arg(source.join("link.ld")).arg(out.join("start32.o")).arg(out.join(format!("{name}.o"))).arg("-o").arg(out.join(format!("{name}.elf"))).status().unwrap().success());
    }
}
