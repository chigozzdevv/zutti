#![allow(clippy::unwrap_used)]
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread::{self, JoinHandle},
    time::Duration,
};
use zcash_protocol::consensus::NetworkType;
use zutti_prover::rpc::SourceRpc;

fn serve(responses: Vec<Value>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 512];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                    let header = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::parse::<usize>)
                        })
                        .unwrap()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let body = serde_json::to_vec(&response).unwrap();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
            stream.write_all(&body).unwrap();
        }
        requests
    });
    (format!("http://{address}"), handle)
}
#[test]
fn rpc_fetches_complete_real_blocks_and_binds_the_requested_network_hash_and_height() {
    let hash = "05a60a92d99d85997cce3b87616c089f6124d7342af37106edc76126334a2c38";
    let bytes = zebra_test::vectors::TESTNET_BLOCKS[&0];
    let (url, server) = serve(vec![
        json!({"id":1,"result":hash,"error":null}),
        json!({"id":1,"result":hex::encode(bytes),"error":null}),
    ]);
    let rpc = SourceRpc::new(url, NetworkType::Test).unwrap();
    assert_eq!(rpc.block(0).unwrap(), bytes);
    let requests = server.join().unwrap();
    assert!(requests[0].contains("getblockhash"));
    assert!(requests[1].contains(hash));
    let (url, server) = serve(vec![
        json!({"id":1,"result":"00".repeat(32)}),
        json!({"id":1,"result":hex::encode(bytes)}),
    ]);
    assert!(
        SourceRpc::new(url, NetworkType::Test)
            .unwrap()
            .block(0)
            .is_err()
    );
    server.join().unwrap();
}
#[test]
fn rpc_errors_wrong_identifiers_and_invalid_heights_never_become_missing_payment_evidence() {
    for response in [
        json!({"id":2,"result":10}),
        json!({"id":1,"error":{"message":"unavailable"},"result":10}),
        json!({"id":1,"result":null}),
        json!({"id":1,"result":-1}),
        json!({"id":1,"result":u64::MAX}),
    ] {
        let (url, server) = serve(vec![response]);
        assert!(
            SourceRpc::new(url, NetworkType::Test)
                .unwrap()
                .height()
                .is_err()
        );
        server.join().unwrap();
    }
    assert!(SourceRpc::new("http://example.com".into(), NetworkType::Test).is_err());
    assert!(SourceRpc::new("https://user:secret@example.com".into(), NetworkType::Test).is_err());
    assert!(SourceRpc::new("https://example.com".into(), NetworkType::Regtest).is_err());
}
#[test]
fn owner_only_rpc_cookies_are_sent_without_exposing_them_in_debug_output() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cookie");
    zutti_wallet::backup::persist_new(&path, b"__cookie__:generated-test-secret").unwrap();
    let (url, server) = serve(vec![json!({"id":1,"result":12})]);
    let rpc = SourceRpc::new(url, NetworkType::Test)
        .unwrap()
        .with_cookie(&path)
        .unwrap();
    assert!(!format!("{rpc:?}").contains("secret"));
    assert_eq!(rpc.height().unwrap(), 12);
    let requests = server.join().unwrap();
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("authorization: basic ")
    );
}
