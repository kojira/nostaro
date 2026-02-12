use anyhow::Result;
use nostr_sdk::prelude::*;

pub fn run(entity: &str) -> Result<()> {
    let nip19 = Nip19::from_bech32(entity)?;

    match nip19 {
        Nip19::Pubkey(pubkey) => {
            println!("Type:   npub (public key)");
            println!("Hex:    {}", pubkey.to_hex());
            println!("Npub:   {}", pubkey.to_bech32()?);
        }
        Nip19::Secret(secret) => {
            println!("Type:   nsec (secret key)");
            println!("Hex:    {}", secret.to_secret_hex());
            println!("Nsec:   exists (hidden for safety)");
        }
        Nip19::EventId(event_id) => {
            println!("Type:   note (event ID)");
            println!("Hex:    {}", event_id.to_hex());
            println!("Note:   {}", event_id.to_bech32()?);
        }
        Nip19::Profile(profile) => {
            println!("Type:   nprofile (profile)");
            println!("Pubkey: {}", profile.public_key.to_hex());
            println!("Npub:   {}", profile.public_key.to_bech32()?);
            if profile.relays.is_empty() {
                println!("Relays: (none)");
            } else {
                println!("Relays:");
                for relay in &profile.relays {
                    println!("  - {}", relay);
                }
            }
        }
        Nip19::Event(event) => {
            println!("Type:     nevent (event)");
            println!("Event ID: {}", event.event_id.to_hex());
            if let Some(author) = event.author {
                println!("Author:   {}", author.to_bech32()?);
            }
            if let Some(kind) = event.kind {
                println!("Kind:     {}", kind.as_u16());
            }
            if event.relays.is_empty() {
                println!("Relays:   (none)");
            } else {
                println!("Relays:");
                for relay in &event.relays {
                    println!("  - {}", relay);
                }
            }
        }
        Nip19::Coordinate(coord) => {
            println!("Type:       naddr (coordinate)");
            println!("Kind:       {}", coord.coordinate.kind.as_u16());
            println!("Pubkey:     {}", coord.coordinate.public_key.to_hex());
            println!("Identifier: {}", coord.coordinate.identifier);
            if coord.relays.is_empty() {
                println!("Relays:     (none)");
            } else {
                println!("Relays:");
                for relay in &coord.relays {
                    println!("  - {}", relay);
                }
            }
        }
    }

    Ok(())
}
