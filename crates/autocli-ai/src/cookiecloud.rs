//! CookieCloud client: fetch and decrypt cookies from a self-hosted CookieCloud server.
//!
//! Supports both encryption modes:
//! - "aes-128-cbc-fixed": key = MD5(uuid+"-"+password)[0..16], IV = 16 zero bytes
//! - "legacy" (CryptoJS): key derived via EVP_BytesToKey from the MD5 password + embedded salt

use std::collections::HashMap;

use aes::Aes128;
use aes::Aes256;
use base64::Engine;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use serde::Deserialize;

use autocli_core::Cookie;

use crate::config::CookieCloudConfig;

type Aes128CbcDec = cbc::Decryptor<Aes128>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;

// ── Wire types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CcResponse {
    encrypted: String,
    #[serde(default)]
    crypto_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CcPayload {
    #[serde(default)]
    cookie_data: HashMap<String, Vec<RawCookie>>,
}

#[derive(Debug, Deserialize)]
struct RawCookie {
    name: String,
    value: String,
    domain: Option<String>,
    path: Option<String>,
    secure: Option<bool>,
    #[serde(rename = "httpOnly")]
    http_only: Option<bool>,
    #[serde(rename = "sameSite")]
    same_site: Option<String>,
    #[serde(rename = "expirationDate")]
    expiration_date: Option<f64>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Fetch and decrypt all cookies from CookieCloud for the given domain.
/// Returns an empty Vec (not an error) if the server has no cookies for that domain.
pub async fn fetch_cookies_for_domain(
    config: &CookieCloudConfig,
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    let payload = fetch_and_decrypt(config).await?;
    Ok(filter_by_domain(&payload, domain))
}

/// Fetch all cookies grouped by domain key (for `cookies list`).
pub async fn fetch_all_cookies(
    config: &CookieCloudConfig,
) -> Result<HashMap<String, Vec<Cookie>>, String> {
    let payload = fetch_and_decrypt(config).await?;
    let mut result: HashMap<String, Vec<Cookie>> = HashMap::new();
    for (key, raw_cookies) in payload.cookie_data {
        let cookies = raw_cookies.iter().map(raw_to_cookie).collect();
        result.insert(key, cookies);
    }
    Ok(result)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

async fn fetch_and_decrypt(config: &CookieCloudConfig) -> Result<CcPayload, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let url = format!(
        "{}/get/{}",
        config.server_url.trim_end_matches('/'),
        config.uuid
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("CookieCloud unreachable: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("CookieCloud returned HTTP {}", resp.status()));
    }

    let cc: CcResponse = resp
        .json()
        .await
        .map_err(|e| format!("Invalid CookieCloud response: {e}"))?;

    let plaintext = decrypt(
        &cc.encrypted,
        &config.uuid,
        &config.password,
        cc.crypto_type.as_deref(),
    )?;

    serde_json::from_slice::<CcPayload>(&plaintext)
        .map_err(|e| format!("Failed to parse decrypted payload: {e}"))
}

fn filter_by_domain(payload: &CcPayload, domain: &str) -> Vec<Cookie> {
    let domain_stripped = domain.trim_start_matches('.');
    let mut cookies = Vec::new();
    for (key, raw_cookies) in &payload.cookie_data {
        let key_stripped = key.trim_start_matches('.');
        // Match: key IS the domain, a subdomain of it, or domain is a subdomain of key
        let matches = key_stripped == domain_stripped
            || key_stripped.ends_with(&format!(".{domain_stripped}"))
            || domain_stripped.ends_with(&format!(".{key_stripped}"));
        if matches {
            cookies.extend(raw_cookies.iter().map(raw_to_cookie));
        }
    }
    cookies
}

fn raw_to_cookie(rc: &RawCookie) -> Cookie {
    Cookie {
        name: rc.name.clone(),
        value: rc.value.clone(),
        domain: rc.domain.clone(),
        path: rc.path.clone(),
        expires: rc.expiration_date,
        http_only: rc.http_only,
        secure: rc.secure,
        same_site: rc.same_site.clone(),
    }
}

// ── Decryption ────────────────────────────────────────────────────────────────

fn decrypt(
    encrypted_b64: &str,
    uuid: &str,
    password: &str,
    crypto_type: Option<&str>,
) -> Result<Vec<u8>, String> {
    match crypto_type {
        Some("aes-128-cbc-fixed") | None => {
            // Try fixed-IV first (newer default); fall back to legacy on failure
            decrypt_fixed_iv(encrypted_b64, uuid, password)
                .or_else(|_| decrypt_legacy(encrypted_b64, uuid, password))
        }
        Some("legacy") => decrypt_legacy(encrypted_b64, uuid, password),
        Some(other) => Err(format!("Unknown CookieCloud crypto_type: {other}")),
    }
}

/// Fixed-IV AES-128-CBC: key = MD5(uuid+"-"+password)[0..16], IV = 0×16
fn decrypt_fixed_iv(encrypted_b64: &str, uuid: &str, password: &str) -> Result<Vec<u8>, String> {
    let key = derive_key_16(uuid, password);
    let iv = [0u8; 16];

    let mut ciphertext = base64::engine::general_purpose::STANDARD
        .decode(encrypted_b64)
        .map_err(|e| format!("base64 decode: {e}"))?;

    let decryptor = Aes128CbcDec::new(&key.into(), &iv.into());
    let plaintext = decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
        .map_err(|e| format!("AES-128-CBC decrypt: {e}"))?;

    Ok(plaintext.to_vec())
}

/// Legacy CryptoJS AES-256-CBC: "Salted__" + 8-byte salt + ciphertext, EVP_BytesToKey key derivation
fn decrypt_legacy(encrypted_b64: &str, uuid: &str, password: &str) -> Result<Vec<u8>, String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encrypted_b64)
        .map_err(|e| format!("base64 decode: {e}"))?;

    if raw.len() < 16 || &raw[..8] != b"Salted__" {
        return Err("Not a CryptoJS salted payload".to_string());
    }

    let salt = &raw[8..16];
    let ciphertext = &raw[16..];

    // Password for EVP_BytesToKey is the 16-char MD5 hex string (as UTF-8 bytes)
    let key_str = derive_key_16_hex(uuid, password);
    let (key32, iv16) = evp_bytes_to_key(key_str.as_bytes(), salt);

    let mut buf = ciphertext.to_vec();
    let decryptor = Aes256CbcDec::new(&key32.into(), &iv16.into());
    let plaintext = decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| format!("AES-256-CBC decrypt: {e}"))?;

    Ok(plaintext.to_vec())
}

// ── Key derivation ────────────────────────────────────────────────────────────

/// Returns the 16-byte AES key: first 16 bytes of the MD5 hex string.
fn derive_key_16(uuid: &str, password: &str) -> [u8; 16] {
    let hex = derive_key_16_hex(uuid, password);
    let mut key = [0u8; 16];
    key.copy_from_slice(hex.as_bytes());
    key
}

/// Returns the first 16 chars of MD5(uuid+"-"+password) as a hex string.
fn derive_key_16_hex(uuid: &str, password: &str) -> String {
    let input = format!("{uuid}-{password}");
    let digest = md5::compute(input.as_bytes());
    format!("{digest:x}")[..16].to_string()
}

/// OpenSSL EVP_BytesToKey with MD5, producing a 32-byte key and 16-byte IV.
fn evp_bytes_to_key(password: &[u8], salt: &[u8]) -> ([u8; 32], [u8; 16]) {
    let mut d: Vec<u8> = Vec::with_capacity(48);
    let mut prev: Vec<u8> = Vec::new();
    while d.len() < 48 {
        let mut input = prev.clone();
        input.extend_from_slice(password);
        input.extend_from_slice(salt);
        let hash = md5::compute(&input);
        prev = hash.0.to_vec();
        d.extend_from_slice(&prev);
    }
    let mut key = [0u8; 32];
    let mut iv = [0u8; 16];
    key.copy_from_slice(&d[..32]);
    iv.copy_from_slice(&d[32..48]);
    (key, iv)
}
