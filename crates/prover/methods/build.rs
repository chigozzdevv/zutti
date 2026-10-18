use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in [
        "../../../Cargo.toml",
        "../../protocol/Cargo.toml",
        "../../protocol/src",
        "../../source/Cargo.toml",
        "../../source/src",
        "../../wallet/Cargo.toml",
        "../../wallet/src",
        "guest/Cargo.toml",
        "guest/Cargo.lock",
        "guest/src",
        "source/Cargo.toml",
        "source/Cargo.lock",
        "source/src",
        "source/vendor",
        "pool/Cargo.toml",
        "pool/Cargo.lock",
        "pool/src",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=RISC0_SKIP_BUILD");
    println!("cargo:rerun-if-env-changed=CARGO_HOME");
    println!("cargo:rerun-if-env-changed=HOME");
    if env::var_os("RISC0_SKIP_BUILD").is_some() {
        return Err("guest builds cannot be skipped".into());
    }
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing manifest directory")?);
    let workspace = manifest.join("../../..").canonicalize()?;
    let default_cargo_home =
        PathBuf::from(env::var_os("HOME").ok_or("missing Cargo home")?).join(".cargo");
    let cargo_home = match env::var_os("CARGO_HOME") {
        Some(path) => PathBuf::from(path),
        None => default_cargo_home.clone(),
    };
    let mut rustc_flags = vec![format!(
        "--remap-path-prefix={}=/zutti",
        workspace.display()
    )];
    for path in [
        cargo_home.clone(),
        cargo_home.canonicalize()?,
        default_cargo_home,
    ] {
        let flag = format!("--remap-path-prefix={}=/cargo", path.display());
        if !rustc_flags.contains(&flag) {
            rustc_flags.push(flag);
        }
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing build output directory")?);
    let target =
        risc0_build::get_target_dir(manifest.join("../../../Cargo.toml")).join("riscv-guest/zutti");
    let mut command = risc0_build::cargo_command("build", &rustc_flags);
    command.env("CARGO_HOME", &cargo_home);
    if !command.get_args().any(|arg| arg == "--locked") {
        command.arg("--locked");
    }
    let status = command
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .arg("--manifest-path")
        .arg(manifest.join("guest/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .arg("--release")
        .status()?;
    if !status.success() {
        return Err("proof guest compilation failed".into());
    }
    let source_target = risc0_build::get_target_dir(manifest.join("../../../Cargo.toml"))
        .join("riscv-guest/zutti-source");
    let status = risc0_build::cargo_command("build", &rustc_flags)
        .env("CARGO_HOME", &cargo_home)
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(manifest.join("source/Cargo.toml"))
        .arg("--target-dir")
        .arg(&source_target)
        .arg("--release")
        .status()?;
    if !status.success() {
        return Err("source proof guest compilation failed".into());
    }
    let pool_target = risc0_build::get_target_dir(manifest.join("../../../Cargo.toml"))
        .join("riscv-guest/zutti-pool");
    let status = risc0_build::cargo_command("build", &rustc_flags)
        .env("CARGO_HOME", &cargo_home)
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(manifest.join("pool/Cargo.toml"))
        .arg("--target-dir")
        .arg(&pool_target)
        .arg("--release")
        .status()?;
    if !status.success() {
        return Err("pool proof guest compilation failed".into());
    }
    let kernel = risc0_build::GuestOptions::default().kernel();
    let mut methods = String::new();
    for (name, constant) in [
        ("history_query", "HISTORY_QUERY"),
        ("preparation", "PREPARATION"),
        ("source", "SOURCE"),
        ("source_update", "SOURCE_UPDATE"),
        ("pool", "POOL"),
    ] {
        let directory = if name.starts_with("source") {
            &source_target
        } else if name == "pool" {
            &pool_target
        } else {
            &target
        };
        let user = fs::read(
            directory
                .join("riscv32im-risc0-zkvm-elf/release")
                .join(name),
        )?;
        let binary = risc0_binfmt::ProgramBinary::new(&user, &kernel).encode();
        let image = risc0_binfmt::compute_image_id(&binary)?;
        let path = out.join(format!("{name}.bin"));
        fs::write(&path, binary)?;
        methods.push_str(&format!(
            "pub const {constant}_ELF: &[u8] = include_bytes!({path:?});\npub const {constant}_ID: [u32; 8] = {:?};\n",
            image.as_words()
        ));
    }
    fs::write(out.join("methods.rs"), methods)?;
    Ok(())
}
