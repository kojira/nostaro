use anyhow::{bail, Result};
use nostr_sdk::prelude::*;

/// Resolve a pubkey string from npub, hex, or nprofile (NIP-19 TLV) format.
pub fn resolve_pubkey(input: &str) -> Result<PublicKey> {
    // Try npub / hex first
    if let Ok(pk) = PublicKey::parse(input) {
        return Ok(pk);
    }

    // Try nprofile (NIP-19 TLV)
    if let Ok(profile) = Nip19Profile::from_bech32(input) {
        return Ok(profile.public_key);
    }

    bail!("Invalid pubkey, npub, or nprofile: {}", input)
}
