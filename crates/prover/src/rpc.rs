use serde_json::{Value, json};
use std::{io::Read, time::Duration};
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType, TEST_NETWORK};
use zutti_protocol::Digest;
use zutti_source::{block::ParsedBlock, bootstrap::GenesisBlock};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_RESPONSE: u64 = 4_001_024;

pub struct SourceRpc {
    agent: ureq::Agent,
    endpoint: String,
    network: NetworkType,
    authorization: Option<zeroize::Zeroizing<String>>,
}
impl std::fmt::Debug for SourceRpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceRpc")
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}
impl SourceRpc {
    pub fn new(endpoint: String, network: NetworkType) -> Result<Self> {
        let uri: ureq::http::Uri = endpoint.parse()?;
        let scheme = uri.scheme_str().ok_or("RPC URL requires a scheme")?;
        let host = uri.host().ok_or("RPC URL requires a host")?;
        if scheme != "https"
            && !(scheme == "http" && matches!(host, "localhost" | "127.0.0.1" | "[::1]"))
        {
            return Err("remote source RPC requires HTTPS".into());
        }
        if uri.authority().is_some_and(|a| a.as_str().contains('@')) {
            return Err("RPC credentials must not be included in its URL".into());
        }
        if !matches!(network, NetworkType::Main | NetworkType::Test) {
            return Err("unsupported source network".into());
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .max_redirects(0)
            .build()
            .into();
        Ok(Self {
            agent,
            endpoint,
            network,
            authorization: None,
        })
    }
    pub fn with_cookie(mut self, path: &std::path::Path) -> Result<Self> {
        use base64::Engine as _;
        let bytes = zutti_wallet::backup::read_private(path, 4096)?;
        let cookie = std::str::from_utf8(&bytes)?.trim_end();
        if !cookie.contains(':') || cookie.contains(['\r', '\n']) {
            return Err("invalid source RPC cookie".into());
        }
        self.authorization = Some(zeroize::Zeroizing::new(format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(cookie)
        )));
        Ok(self)
    }
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut request = self.agent.post(&self.endpoint);
        if let Some(authorization) = &self.authorization {
            request = request.header("Authorization", authorization.as_str());
        }
        let mut response = request
            .send_json(json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .map_err(|_| "source RPC transport failed")?;
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "source RPC response failed")?;
        if bytes.len() as u64 > MAX_RESPONSE {
            return Err("source RPC response exceeds its bound".into());
        }
        let response: Value = serde_json::from_slice(&bytes)?;
        if response["id"] != 1 || !response["error"].is_null() {
            return Err("source RPC returned an error or mismatched request identifier".into());
        }
        response
            .get("result")
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or_else(|| "source RPC result is missing".into())
    }
    pub fn height(&self) -> Result<u32> {
        let height = self.call("getblockcount", json!([]))?;
        Ok(u32::try_from(
            height.as_u64().ok_or("invalid source RPC height")?,
        )?)
    }
    pub fn hash(&self, height: u32) -> Result<Digest> {
        let value = self.call("getblockhash", json!([height]))?;
        let mut hash = Digest::from_hex(value.as_str().ok_or("invalid source RPC block hash")?)?;
        hash.0.reverse();
        Ok(hash)
    }
    pub fn block(&self, height: u32) -> Result<Vec<u8>> {
        zutti_source::upgrade::consensus_branch(self.network, height)?;
        let hash = self.hash(height)?;
        let mut display = hash.0;
        display.reverse();
        let value = self.call("getblock", json!([hex::encode(display), 0]))?;
        let bytes = hex::decode(
            value
                .as_str()
                .ok_or("source RPC must return a serialized block")?,
        )?;
        let block = match self.network {
            NetworkType::Main => ParsedBlock::parse(&bytes, &MAIN_NETWORK)?,
            NetworkType::Test => ParsedBlock::parse(&bytes, &TEST_NETWORK)?,
            _ => return Err("unsupported source network".into()),
        };
        if block.hash() != hash || block.claimed_height() != height {
            return Err("source RPC block differs from its requested hash or height".into());
        }
        if height == 0 {
            GenesisBlock::parse(&bytes, self.network)?;
        }
        Ok(bytes)
    }
}
