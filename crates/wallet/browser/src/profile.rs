use serde_json::Value;
use zutti_ethereum::{ConsensusConfig, spv_authorization::SpvAuthorizationProfile};
use zutti_protocol::{Digest, domain::Domain, encoding::Canonical};
use zutti_source::spv::Policy;

pub fn profile() -> Result<SpvAuthorizationProfile, String> {
    let deployment: Value = serde_json::from_str(include_str!(
        "../../../../deploy/ethereum/optimized/sepolia.json"
    ))
    .map_err(|_| "invalid embedded deployment")?;
    let text = |key: &str| deployment[key].as_str().ok_or("missing deployment field");
    let domain = Domain::from_bytes(&hex::decode(text("domain_encoding")?).map_err(|_| "domain")?)
        .map_err(|_| "domain")?;
    let policy = Policy::from_bytes(&hex::decode(text("source_policy")?).map_err(|_| "policy")?)
        .map_err(|_| "policy")?;
    let ethereum: ConsensusConfig = serde_json::from_str(include_str!(
        "../../../../deploy/ethereum/optimized/sepolia-consensus.json"
    ))
    .map_err(|_| "invalid embedded consensus")?;
    if deployment["network"] != "testnet"
        || deployment["chain_id"] != 11155111
        || !policy.paid_only()
        || domain.source_genesis() != zutti_protocol::network::ZCASH_TESTNET_GENESIS
        || domain.destination_chain() != 11155111
        || ethereum.chain_id != 11155111
        || domain.source_policy() != policy.id().map_err(|_| "policy")?
        || domain.id().map_err(|_| "domain")?.to_string() != text("domain")?
        || policy.program().to_string() != text("source_image")?
    {
        return Err("embedded wallet profile mismatch".into());
    }
    Ok(SpvAuthorizationProfile {
        domain,
        ethereum,
        pool_code_hash: deployment["code_hashes"]["pool"]
            .as_str()
            .ok_or("pool code hash")?
            .parse()
            .map_err(|_| "pool code hash")?,
        source: text("registry")?.parse().map_err(|_| "registry")?,
        source_code_hash: deployment["code_hashes"]["registry"]
            .as_str()
            .ok_or("registry code hash")?
            .parse()
            .map_err(|_| "registry code hash")?,
        binding_program: Digest::from_hex(text("binding_image")?).map_err(|_| "binding")?,
        pool_program: Digest::from_hex(text("pool_image")?).map_err(|_| "pool")?,
        policy,
        minimum_blocks: 64,
        maximum_source_age: 3600,
        future_drift: 300,
    })
}
