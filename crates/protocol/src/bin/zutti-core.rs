use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use zeroize::Zeroizing;
use zutti_protocol::{
    Error, Result,
    amount::{Amount, Asset, FeeBps},
};

#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Amount {
        asset: String,
        amount: String,
        positive: bool,
    },
    Compare {
        asset: String,
        left: String,
        right: String,
    },
    Net {
        asset: String,
        gross: String,
        fee_bps: u16,
    },
    Quote {
        from: String,
        to: String,
        amount: String,
        rate: String,
        fee_bps: u16,
    },
}
fn asset(value: &str) -> Result<Asset> {
    match value {
        "ZEC" => Ok(Asset::Zec),
        "USDC" => Ok(Asset::Usdc),
        "ETH" => Ok(Asset::Eth),
        _ => Err(Error::Asset),
    }
}
fn result(input: Amount, gross: Amount, fee: FeeBps) -> Result<Value> {
    let net = gross.net_after_fee(fee).positive()?;
    Ok(
        json!({"input":input.to_string(), "gross":gross.to_string(), "net":net.to_string(), "fee":gross.checked_sub(net)?.to_string()}),
    )
}
fn evaluate(request: Request) -> Result<Value> {
    match request {
        Request::Amount {
            asset: name,
            amount,
            positive,
        } => {
            let parsed = Amount::parse(asset(&name)?, &amount)?;
            if positive {
                parsed.positive()?;
            }
            Ok(json!({"amount":parsed.to_string(), "units":parsed.units().to_string()}))
        }
        Request::Compare {
            asset: name,
            left,
            right,
        } => {
            let asset = asset(&name)?;
            let left = Amount::parse(asset, &left)?.units();
            let right = Amount::parse(asset, &right)?.units();
            Ok(
                json!({"ordering":match left.cmp(&right) { std::cmp::Ordering::Less => -1, std::cmp::Ordering::Equal => 0, std::cmp::Ordering::Greater => 1 }}),
            )
        }
        Request::Net {
            asset: name,
            gross,
            fee_bps,
        } => {
            let gross = Amount::parse(asset(&name)?, &gross)?.positive()?;
            result(gross, gross, FeeBps::new(fee_bps)?)
        }
        Request::Quote {
            from,
            to,
            amount,
            rate,
            fee_bps,
        } => {
            let from = asset(&from)?;
            let to = asset(&to)?;
            let input = Amount::parse(from, &amount)?.positive()?;
            let rate = Amount::parse(Asset::Usdc, &rate)?.positive()?.units();
            let units = match (from, to) {
                (Asset::Zec, Asset::Usdc) => {
                    input.units().checked_mul(rate).ok_or(Error::Overflow)? / 100_000_000
                }
                (Asset::Usdc, Asset::Zec) => {
                    input
                        .units()
                        .checked_mul(100_000_000)
                        .ok_or(Error::Overflow)?
                        / rate
                }
                _ => return Err(Error::Asset),
            };
            result(
                input,
                Amount::from_units(to, units)?.positive()?,
                FeeBps::new(fee_bps)?,
            )
        }
    }
}
fn run() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut bytes = Zeroizing::new(Vec::new());
    input.by_ref().take(16_385).read_to_end(&mut bytes)?;
    let response = if bytes.len() > 16_384 {
        json!({"version":1,"ok":false,"error":"invalid_input"})
    } else {
        match serde_json::from_slice::<Request>(&bytes)
            .ok()
            .and_then(|request| evaluate(request).ok())
        {
            Some(value) => json!({"version":1,"ok":true,"result":value}),
            None => json!({"version":1,"ok":false,"error":"invalid_input"}),
        }
    };
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, &response)?;
    output.write_all(b"\n")
}
fn main() -> io::Result<()> {
    run()
}
