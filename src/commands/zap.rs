use std::process::Command;

use anyhow::{anyhow, bail, Result};
use nostr_sdk::prelude::*;
use serde::Deserialize;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::utils::resolve_pubkey;

#[derive(Deserialize)]
struct LnurlResponse {
    callback: String,
    #[serde(rename = "minSendable")]
    min_sendable: Option<u64>,
    #[serde(rename = "maxSendable")]
    max_sendable: Option<u64>,
    #[serde(rename = "allowsNostr")]
    allows_nostr: Option<bool>,
}

#[derive(Deserialize)]
struct InvoiceResponse {
    pr: String,
}

pub async fn run(target: &str, amount: u64, message: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let target_pubkey = resolve_pubkey(target)?;

    let metadata = client::fetch_profile(&nostr_client, &target_pubkey)
        .await?
        .ok_or_else(|| anyhow!("Could not find profile for {}", target))?;

    let lnurl_endpoint = resolve_lnurl(&metadata)?;

    println!("Fetching LNURL endpoint...");
    let http_client = reqwest::Client::new();
    let lnurl_resp: LnurlResponse = http_client
        .get(&lnurl_endpoint)
        .send()
        .await?
        .json()
        .await?;

    if lnurl_resp.allows_nostr != Some(true) {
        bail!("Target's LNURL endpoint does not support Nostr zaps");
    }

    let amount_msats = amount * 1000;

    if let Some(min) = lnurl_resp.min_sendable {
        if amount_msats < min {
            bail!("Amount too small. Minimum: {} sats", min / 1000);
        }
    }
    if let Some(max) = lnurl_resp.max_sendable {
        if amount_msats > max {
            bail!("Amount too large. Maximum: {} sats", max / 1000);
        }
    }

    let content = message.unwrap_or("");
    let relays = config.active_relays();
    let mut relay_tag_values: Vec<&str> = vec!["relays"];
    for r in &relays {
        relay_tag_values.push(r.as_str());
    }

    let tags = vec![
        Tag::public_key(target_pubkey),
        Tag::parse(["amount".to_string(), amount_msats.to_string()])?,
        Tag::parse(relay_tag_values.into_iter().map(|s| s.to_string()).collect::<Vec<String>>())?,
    ];

    let builder = EventBuilder::new(Kind::ZapRequest, content).tags(tags);
    let zap_request = builder.sign_with_keys(&keys)?;
    let zap_request_json = zap_request.as_json();

    let invoice_resp: InvoiceResponse = http_client
        .get(&lnurl_resp.callback)
        .query(&[
            ("amount", amount_msats.to_string()),
            ("nostr", zap_request_json),
        ])
        .send()
        .await?
        .json()
        .await?;

    let target_npub = target_pubkey.to_bech32()?;

    println!("Paying invoice via Cashu...");
    let output = Command::new("/Users/kojira/.openclaw/workspace/data/cashu-venv/bin/cashu")
        .args(["-h", "https://mint.coinos.io", "pay", &invoice_resp.pr, "-y"])
        .output()?;

    if output.status.success() {
        println!(
            "⚡ Zap sent successfully! {} sats to {}",
            amount,
            &target_npub[..12.min(target_npub.len())]
        );
    } else {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Cashu payment failed:\n{}{}", stdout, stderr);
    }

    Ok(())
}

fn resolve_lnurl(metadata: &Metadata) -> Result<String> {
    if let Some(ref lud16) = metadata.lud16 {
        let parts: Vec<&str> = lud16.split('@').collect();
        if parts.len() == 2 {
            return Ok(format!(
                "https://{}/.well-known/lnurlp/{}",
                parts[1], parts[0]
            ));
        }
    }

    if let Some(ref lud06) = metadata.lud06 {
        if lud06.starts_with("http") {
            return Ok(lud06.clone());
        }
        bail!("LNURL bech32 decoding not yet supported. Use a Lightning address (lud16) instead.");
    }

    bail!("Target has no Lightning address configured (lud06/lud16)");
}
