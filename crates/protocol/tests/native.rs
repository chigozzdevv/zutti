#![cfg(feature = "native")]
#![allow(clippy::unwrap_used)]
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn call(value: &str) -> Value {
    let mut process = Command::new(env!("CARGO_BIN_EXE_zutti-core"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(value.as_bytes())
        .unwrap();
    let output = process.wait_with_output().unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn native_bridge_preserves_precision_and_directional_fees() {
    assert_eq!(
        call(r#"{"method":"amount","asset":"ZEC","amount":"5.00000000","positive":true}"#)["result"],
        json!({"amount":"5","units":"500000000"})
    );
    for (from, to, amount, gross, net, fee) in [
        ("ZEC", "USDC", "5", "200", "199", "1"),
        ("USDC", "ZEC", "100", "2.5", "2.4875", "0.0125"),
    ] {
        let request =
            json!({"method":"quote","from":from,"to":to,"amount":amount,"rate":"40","fee_bps":50});
        assert_eq!(
            call(&request.to_string())["result"],
            json!({"input":amount,"gross":gross,"net":net,"fee":fee})
        );
    }
}
#[test]
fn native_bridge_rejects_malformed_precision_overflow_and_unbounded_requests() {
    for value in [
        r#"{"method":"amount","asset":"USDC","amount":"1.0000001","positive":true}"#,
        r#"{"method":"amount","asset":"ZEC","amount":"21000001","positive":true}"#,
        r#"{"method":"net","asset":"USDC","gross":"1","fee_bps":10000}"#,
        r#"{"method":"amount","asset":"ZEC","amount":"5","positive":true,"ignored":true}"#,
        "{} {}",
    ] {
        assert_eq!(
            call(value),
            json!({"version":1,"ok":false,"error":"invalid_input"})
        );
    }
    assert_eq!(call(&" ".repeat(16_385))["ok"], false);
}
