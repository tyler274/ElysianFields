//! Pure-Rust SHA-256, HMAC-SHA-256, PBKDF2-HMAC-SHA-256, Base64, and RFC 5802 / RFC 7677
//! `SCRAM-SHA-256` client authentication state machine with zero external crypto crates.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::PgError;

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const SHA256_IV: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// Computes the FIPS 180-4 SHA-256 digest of `data`.
#[must_use]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state = SHA256_IV;
    let bit_len = (data.len() as u64).wrapping_mul(8);

    let (full_blocks, rem) = data.as_chunks::<64>();
    for block in full_blocks {
        compress_block(&mut state, block);
    }

    let mut padded = [0u8; 128];
    padded[..rem.len()].copy_from_slice(rem);
    padded[rem.len()] = 0x80;

    let final_len = if rem.len() < 56 { 64 } else { 128 };
    padded[final_len - 8..final_len].copy_from_slice(&bit_len.to_be_bytes());

    let (final_blocks, _) = padded[..final_len].as_chunks::<64>();
    for block in final_blocks {
        compress_block(&mut state, block);
    }

    let mut out = [0u8; 32];
    for (idx, word) in state.iter().enumerate() {
        out[idx * 4..(idx + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn compress_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (i, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_be_bytes(*chunk);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(SHA256_K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// Computes RFC 2104 HMAC-SHA-256.
#[must_use]
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hashed = sha256(key);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = Vec::with_capacity(64 + data.len());
    let mut opad = [0u8; 96];
    for i in 0..64 {
        ipad.push(key_block[i] ^ 0x36);
        opad[i] = key_block[i] ^ 0x5c;
    }
    ipad.extend_from_slice(data);

    let inner = sha256(&ipad);
    opad[64..96].copy_from_slice(&inner);
    sha256(&opad)
}

/// Computes single-block (32-byte) RFC 2898 PBKDF2-HMAC-SHA-256 (`dkLen = 32`).
#[must_use]
pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut salt_block = Vec::with_capacity(salt.len() + 4);
    salt_block.extend_from_slice(salt);
    salt_block.extend_from_slice(&1u32.to_be_bytes());

    let mut u = hmac_sha256(password, &salt_block);
    let mut out = u;
    for _ in 1..iterations {
        u = hmac_sha256(password, &u);
        for (o, b) in out.iter_mut().zip(u.iter()) {
            *o ^= *b;
        }
    }
    out
}

/// Constant-time byte-slice equality check for authentication signatures.
#[must_use]
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard RFC 4648 Base64 encoding with `=` padding.
#[must_use]
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let (chunks, rem) = input.as_chunks::<3>();
    for &[a, b, c] in chunks {
        out.push(B64_ALPHABET[(a >> 2) as usize] as char);
        out.push(B64_ALPHABET[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
        out.push(B64_ALPHABET[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char);
        out.push(B64_ALPHABET[(c & 0x3f) as usize] as char);
    }
    match rem {
        [a] => {
            out.push(B64_ALPHABET[(a >> 2) as usize] as char);
            out.push(B64_ALPHABET[((a & 0x03) << 4) as usize] as char);
            out.push('=');
            out.push('=');
        }
        [a, b] => {
            out.push(B64_ALPHABET[(a >> 2) as usize] as char);
            out.push(B64_ALPHABET[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
            out.push(B64_ALPHABET[((b & 0x0f) << 2) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

/// Standard RFC 4648 Base64 decoding.
pub fn base64_decode(input: &str) -> Result<Vec<u8>, PgError> {
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(PgError::Auth {
            detail: "invalid Base64 length".to_owned(),
        });
    }
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for quad in bytes.as_chunks::<4>().0 {
        let v0 = b64_val(quad[0])?;
        let v1 = b64_val(quad[1])?;
        out.push((v0 << 2) | (v1 >> 4));
        if quad[2] != b'=' {
            let v2 = b64_val(quad[2])?;
            out.push((v1 << 4) | (v2 >> 2));
            if quad[3] != b'=' {
                let v3 = b64_val(quad[3])?;
                out.push((v2 << 6) | v3);
            }
        } else if quad[3] != b'=' {
            return Err(PgError::Auth {
                detail: "invalid Base64 padding".to_owned(),
            });
        }
    }
    Ok(out)
}

fn b64_val(b: u8) -> Result<u8, PgError> {
    match b {
        b'A'..=b'Z' => Ok(b - b'A'),
        b'a'..=b'z' => Ok(b - b'a' + 26),
        b'0'..=b'9' => Ok(b - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(PgError::Auth {
            detail: format!("invalid Base64 byte 0x{b:02x}"),
        }),
    }
}

static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generates a unique 192-bit printable client nonce for SCRAM-SHA-256.
#[must_use]
pub fn generate_client_nonce() -> String {
    let seq = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut seed = Vec::with_capacity(32);
    seed.extend_from_slice(&seq.to_le_bytes());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0u128, |d| d.as_nanos());
        seed.extend_from_slice(&nanos.to_le_bytes());
        seed.extend_from_slice(&std::process::id().to_le_bytes());
    }
    let digest = sha256(&seed);
    base64_encode(&digest[..24])
}

/// RFC 5802 / RFC 7677 `SCRAM-SHA-256` client handshake state machine.
pub struct ScramSha256Client {
    username: String,
    password: String,
    client_nonce: String,
    client_first_bare: String,
    expected_server_sig: Option<[u8; 32]>,
}

impl ScramSha256Client {
    #[must_use]
    pub fn new(username: &str, password: &str, client_nonce: String) -> Self {
        let escaped_user = escape_scram_username(username);
        let client_first_bare = format!("n={escaped_user},r={client_nonce}");
        Self {
            username: username.to_owned(),
            password: password.to_owned(),
            client_nonce,
            client_first_bare,
            expected_server_sig: None,
        }
    }

    /// Returns the SASL `client-first-message` (`n,,n=<user>,r=<nonce>`).
    #[must_use]
    pub fn client_first_message(&self) -> String {
        let _ = &self.username;
        format!("n,,{}", self.client_first_bare)
    }

    /// Validates the server's `server-first-message` and computes the `client-final-message`.
    pub fn process_server_first(&mut self, server_first: &str) -> Result<String, PgError> {
        let mut combined_nonce: Option<&str> = None;
        let mut salt_b64: Option<&str> = None;
        let mut iterations: Option<u32> = None;

        for part in server_first.split(',') {
            if let Some(val) = part.strip_prefix("r=") {
                combined_nonce = Some(val);
            } else if let Some(val) = part.strip_prefix("s=") {
                salt_b64 = Some(val);
            } else if let Some(val) = part.strip_prefix("i=") {
                iterations = Some(val.parse::<u32>().map_err(|err| PgError::Auth {
                    detail: format!("invalid SCRAM iteration count '{val}': {err}"),
                })?);
            }
        }

        let r = combined_nonce.ok_or_else(|| PgError::Auth {
            detail: "SCRAM server-first missing nonce (r=)".to_owned(),
        })?;
        if !r.starts_with(&self.client_nonce) || r.len() <= self.client_nonce.len() {
            return Err(PgError::Auth {
                detail: "SCRAM server nonce does not extend client nonce".to_owned(),
            });
        }
        let salt = base64_decode(salt_b64.ok_or_else(|| PgError::Auth {
            detail: "SCRAM server-first missing salt (s=)".to_owned(),
        })?)?;
        let iter = iterations.ok_or_else(|| PgError::Auth {
            detail: "SCRAM server-first missing iterations (i=)".to_owned(),
        })?;
        if iter == 0 || iter > 1_000_000 {
            return Err(PgError::Auth {
                detail: format!("SCRAM iteration count out of safe bounds: {iter}"),
            });
        }

        let salted_password = pbkdf2_hmac_sha256(self.password.as_bytes(), &salt, iter);
        let client_key = hmac_sha256(&salted_password, b"Client Key");
        let stored_key = sha256(&client_key);

        let client_final_without_proof = format!("c=biws,r={r}");
        let auth_message = format!(
            "{},{server_first},{client_final_without_proof}",
            self.client_first_bare
        );

        let client_signature = hmac_sha256(&stored_key, auth_message.as_bytes());
        let mut client_proof = [0u8; 32];
        for i in 0..32 {
            client_proof[i] = client_key[i] ^ client_signature[i];
        }

        let server_key = hmac_sha256(&salted_password, b"Server Key");
        let server_signature = hmac_sha256(&server_key, auth_message.as_bytes());
        self.expected_server_sig = Some(server_signature);

        Ok(format!(
            "{client_final_without_proof},p={}",
            base64_encode(&client_proof)
        ))
    }

    /// Verifies the server's `server-final-message` (`v=<server_signature>`).
    pub fn verify_server_final(&self, server_final: &str) -> Result<(), PgError> {
        let expected = self.expected_server_sig.ok_or_else(|| PgError::Auth {
            detail: "SCRAM verify_server_final called before process_server_first".to_owned(),
        })?;
        for part in server_final.split(',') {
            if let Some(err_val) = part.strip_prefix("e=") {
                return Err(PgError::Auth {
                    detail: format!("SCRAM server rejected authentication: {err_val}"),
                });
            }
            if let Some(sig_b64) = part.strip_prefix("v=") {
                let sig_bytes = base64_decode(sig_b64)?;
                if ct_eq(&sig_bytes, &expected) {
                    return Ok(());
                }
                return Err(PgError::Auth {
                    detail: "SCRAM server signature verification failed".to_owned(),
                });
            }
        }
        Err(PgError::Auth {
            detail: "SCRAM server-final missing verifier (v=)".to_owned(),
        })
    }
}

fn escape_scram_username(username: &str) -> String {
    let mut out = String::with_capacity(username.len());
    for ch in username.chars() {
        match ch {
            '=' => out.push_str("=3D"),
            ',' => out.push_str("=2C"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    #[test]
    fn sha256_matches_fips_180_4_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hmac_and_scram_sha256_match_rfc_7677_vector() {
        // Official RFC 7677 Section 3 SCRAM-SHA-256 test vector.
        let mut client =
            ScramSha256Client::new("user", "pencil", "rOprNGfwEbeRWgbNEkqO".to_owned());
        assert_eq!(
            client.client_first_message(),
            "n,,n=user,r=rOprNGfwEbeRWgbNEkqO"
        );
        let server_first = "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";
        let client_final = client
            .process_server_first(server_first)
            .expect("process server-first");
        assert_eq!(
            client_final,
            "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
        client
            .verify_server_final("v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=")
            .expect("verify RFC 7677 server signature");
    }
}
