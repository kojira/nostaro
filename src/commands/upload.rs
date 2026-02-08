use anyhow::{bail, Result};
use base64::Engine;
use nostr_sdk::prelude::*;
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(file_path: &str, server: Option<&str>, nip96: bool) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;

    let path = Path::new(file_path);
    if !path.exists() {
        bail!("File not found: {}", file_path);
    }

    let file_data = std::fs::read(path)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "upload".to_string());

    if nip96 {
        upload_nip96(&keys, &file_data, &file_name, server).await
    } else {
        upload_blossom(&keys, &file_data, &file_name, server, &config).await
    }
}

async fn upload_blossom(
    keys: &Keys,
    data: &[u8],
    file_name: &str,
    server: Option<&str>,
    config: &NostaroConfig,
) -> Result<()> {
    let blossom_url = server
        .map(|s| s.to_string())
        .unwrap_or_else(|| config.blossom_url());

    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();

    println!(
        "Uploading {} ({} bytes) to {}...",
        file_name,
        data.len(),
        blossom_url
    );

    let now = Timestamp::now();
    let tags = vec![
        Tag::parse(["t", "upload"])?,
        Tag::parse(["x", &hash])?,
        Tag::parse(["expiration", &(now.as_u64() + 300).to_string()])?,
    ];
    let builder = EventBuilder::new(Kind::Custom(24242), "Upload").tags(tags);
    let auth_event = builder.sign_with_keys(keys)?;
    let auth_json = auth_event.as_json();
    let auth_base64 = base64::engine::general_purpose::STANDARD.encode(auth_json.as_bytes());

    let content_type = mime_type_from_ext(file_name);

    let http_client = reqwest::Client::new();
    let resp = http_client
        .put(format!("{}/upload", blossom_url))
        .header("Authorization", format!("Nostr {}", auth_base64))
        .header("Content-Type", content_type)
        .body(data.to_vec())
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Upload failed ({}): {}", status, body);
    }

    let resp_json: serde_json::Value = resp.json().await?;
    if let Some(url) = resp_json.get("url").and_then(|v| v.as_str()) {
        println!("Uploaded successfully!");
        println!("URL: {}", url);
    } else {
        println!(
            "Upload response: {}",
            serde_json::to_string_pretty(&resp_json)?
        );
    }

    Ok(())
}

async fn upload_nip96(
    keys: &Keys,
    data: &[u8],
    file_name: &str,
    server: Option<&str>,
) -> Result<()> {
    let server_url = server.unwrap_or("https://nostr.build");

    println!(
        "Uploading {} ({} bytes) via NIP-96 to {}...",
        file_name,
        data.len(),
        server_url
    );

    let http_client = reqwest::Client::new();

    let well_known_url = format!("{}/.well-known/nostr/nip96.json", server_url);
    let well_known: serde_json::Value =
        http_client.get(&well_known_url).send().await?.json().await?;

    let api_url = well_known
        .get("api_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("No api_url in NIP-96 well-known"))?;

    let upload_url = if api_url.starts_with("http") {
        api_url.to_string()
    } else {
        format!("{}{}", server_url, api_url)
    };

    let tags = vec![
        Tag::parse(["u", &upload_url])?,
        Tag::parse(["method", "POST"])?,
    ];
    let builder = EventBuilder::new(Kind::Custom(27235), "").tags(tags);
    let auth_event = builder.sign_with_keys(keys)?;
    let auth_json = auth_event.as_json();
    let auth_base64 = base64::engine::general_purpose::STANDARD.encode(auth_json.as_bytes());

    let content_type = mime_type_from_ext(file_name);
    let part = reqwest::multipart::Part::bytes(data.to_vec())
        .file_name(file_name.to_string())
        .mime_str(&content_type)?;

    let form = reqwest::multipart::Form::new().part("file", part);

    let resp = http_client
        .post(&upload_url)
        .header("Authorization", format!("Nostr {}", auth_base64))
        .multipart(form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Upload failed ({}): {}", status, body);
    }

    let resp_json: serde_json::Value = resp.json().await?;
    if let Some(tags) = resp_json.pointer("/nip94_event/tags") {
        if let Some(arr) = tags.as_array() {
            for tag in arr {
                if let Some(tag_arr) = tag.as_array() {
                    if tag_arr.first().and_then(|v| v.as_str()) == Some("url") {
                        if let Some(url) = tag_arr.get(1).and_then(|v| v.as_str()) {
                            println!("Uploaded successfully!");
                            println!("URL: {}", url);
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    println!(
        "Upload response: {}",
        serde_json::to_string_pretty(&resp_json)?
    );
    Ok(())
}

fn mime_type_from_ext(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
    .to_string()
}
