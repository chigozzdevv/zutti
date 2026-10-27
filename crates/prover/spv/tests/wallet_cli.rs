use zutti_protocol::encoding::Canonical;
use zutti_wallet::{account::WalletAccount, spv_pool::SpvPoolWitness};

#[test]
fn deposit_cli_prepares_the_deployed_domain_without_spending_tokens() {
    use std::{fs, process::Command};
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("account");
    let account = WalletAccount::create(&directory).unwrap();
    let output = root.path().join("deposit");
    let deployment = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../deploy/ethereum/sepolia.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&deployment).unwrap()).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_zutti-wallet"))
            .arg("deposit")
            .arg("--directory")
            .arg(&directory)
            .arg("--deployment")
            .arg(&deployment)
            .args(["--amount", "10.25", "--out"])
            .arg(&output)
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = fs::read(output.join("witness.bin")).unwrap();
    let witness = SpvPoolWitness::from_bytes(&bytes).unwrap();
    assert_eq!(witness.claim.pool.domain.to_string(), manifest["domain"]);
    assert_eq!(witness.claim.pool.value, 10_250_000);
    assert_eq!(
        witness.outputs[0].note.recipient(),
        account.private_receiver().unwrap()
    );
    let raw = zutti_wallet::backup::read_private(&directory.join("account.key"), 32).unwrap();
    let key = zutti_protocol::Secret32::from_bytes(raw.as_slice().try_into().unwrap()).unwrap();
    let archive = fs::read(output.join("deposit.backup")).unwrap();
    assert_eq!(
        zutti_wallet::backup::open(witness.claim.pool.domain, &archive, &key)
            .unwrap()
            .as_slice(),
        bytes
    );
    assert!(!run().status.success());
    assert_eq!(fs::read(output.join("witness.bin")).unwrap(), bytes);
    let ciphertext = root.path().join("note.bin");
    fs::write(
        &ciphertext,
        witness.outputs[0].encrypted().unwrap().to_bytes().unwrap(),
    )
    .unwrap();
    let inspect = |account: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_zutti-wallet"))
            .arg("note")
            .arg("--directory")
            .arg(account)
            .arg("--deployment")
            .arg(&deployment)
            .arg("--ciphertext")
            .arg(&ciphertext)
            .output()
            .unwrap()
    };
    let result = inspect(&directory);
    assert!(result.status.success());
    let note: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(note["owned"], true);
    assert_eq!(note["amount_units"], 10_250_000);
    assert_eq!(note["membership_verified"], false);
    let other = root.path().join("other");
    WalletAccount::create(&other).unwrap();
    assert!(!inspect(&other).status.success());
}
