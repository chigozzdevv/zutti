use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err("profile input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("profile input exceeds its bound".into());
    }
    Ok(bytes)
}

pub fn program(binary: &str, path: Option<&Path>) -> Result<Vec<u8>> {
    if let Some(path) = path {
        return bounded(path, 32 * 1024 * 1024);
    }
    if ![
        "zutti-validation-profile",
        "zutti-tree-profile",
        "zutti-validation-batch",
        "context",
    ]
    .contains(&binary)
    {
        return Err("unsupported diagnostic guest".into());
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let target =
        risc0_build::get_target_dir(root.join("Cargo.toml")).join("riscv-guest/zutti-source");
    let cargo_home =
        PathBuf::from(std::env::var_os("CARGO_HOME").ok_or("set CARGO_HOME explicitly")?);
    let mut flags = vec![format!(
        "--remap-path-prefix={}=/zutti",
        root.canonicalize()?.display()
    )];
    for path in [
        cargo_home.clone(),
        cargo_home.canonicalize()?,
        PathBuf::from(std::env::var_os("HOME").ok_or("missing home")?).join(".cargo"),
    ] {
        let flag = format!("--remap-path-prefix={}=/cargo", path.display());
        if !flags.contains(&flag) {
            flags.push(flag);
        }
    }
    let status = risc0_build::cargo_command("build", &flags)
        .env("CARGO_HOME", &cargo_home)
        .env("CARGO_NET_OFFLINE", "true")
        .args([
            "--locked",
            "--offline",
            "--release",
            "--bin",
            binary,
            "-j",
            "1",
            "--manifest-path",
        ])
        .arg(root.join("benchmarks/validation/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .status()?;
    if !status.success() {
        return Err("diagnostic guest compilation failed".into());
    }
    let user = bounded(
        &target.join("riscv32im-risc0-zkvm-elf/release").join(binary),
        32 * 1024 * 1024,
    )?;
    Ok(
        risc0_binfmt::ProgramBinary::new(&user, &risc0_build::GuestOptions::default().kernel())
            .encode(),
    )
}
