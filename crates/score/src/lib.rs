//! What the leaderboard client and `score-server` must agree on (TIME-TRIAL-RELIC-LEADERBOARDS.md):
//! how a write request is signed by the player key, and how that key is shown and moved. One
//! copy for both, so they can't drift; the test pins the bytes against an independent
//! implementation (`score_sign.py`).
//!
//! The key types are iroh's (`net::SecretKey` is the same type), which sign and generate on
//! native and wasm alike. The player key is never an endpoint key, though.

use data_encoding::{HEXLOWER, HEXLOWER_PERMISSIVE};
pub use iroh_base::{PublicKey, SecretKey, Signature};
use sha2::{Digest, Sha256};

/// The `Authorization` scheme.
pub const SCHEME: &str = "CTR-Ed25519";
/// Leads the signed bytes: a signature for anything else can't be passed off as a request's.
const DOMAIN: &str = "ctr-score/1";
/// How far a request's `ts` may be from the server's clock, either way.
pub const MAX_SKEW: u64 = 300;

pub const NICK_HEADER: &str = "X-CTR-Nick";
pub const VERSION_HEADER: &str = "X-CTR-Version";
pub const CLAIMED_MS_HEADER: &str = "X-CTR-Claimed-Ms";

/// What a request's signature is over. `path` is without the query: writes have none, and a
/// proxy may re-encode one.
pub fn message(method: &str, path: &str, ts: u64, body: &[u8]) -> Vec<u8> {
    let digest = HEXLOWER.encode(&Sha256::digest(body));
    format!("{DOMAIN}\n{method}\n{path}\n{ts}\n{digest}").into_bytes()
}

/// A signed request's `Authorization`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Auth {
    pub key: PublicKey,
    /// Unix seconds, by the client's clock (plus the offset a `401` told it).
    pub ts: u64,
    pub sig: Signature,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    Malformed(&'static str),
    BadSignature,
    Stale,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Malformed(why) => write!(f, "malformed authorization: {why}"),
            AuthError::BadSignature => f.write_str("bad signature"),
            AuthError::Stale => f.write_str("stale timestamp"),
        }
    }
}

impl Auth {
    pub fn sign(secret: &SecretKey, method: &str, path: &str, ts: u64, body: &[u8]) -> Auth {
        Auth {
            key: secret.public(),
            ts,
            sig: secret.sign(&message(method, path, ts, body)),
        }
    }

    pub fn header(&self) -> String {
        format!(
            "{SCHEME} key={}, ts={}, sig={}",
            HEXLOWER.encode(self.key.as_bytes()),
            self.ts,
            HEXLOWER.encode(&self.sig.to_bytes())
        )
    }

    pub fn parse(header: &str) -> Result<Auth, AuthError> {
        let params = header
            .strip_prefix(SCHEME)
            .and_then(|rest| rest.strip_prefix(' '))
            .ok_or(AuthError::Malformed("not the scheme"))?;
        let (mut key, mut ts, mut sig) = (None, None, None);
        for param in params.split(',') {
            let (name, value) = param
                .trim()
                .split_once('=')
                .ok_or(AuthError::Malformed("a parameter without a value"))?;
            let slot = match name {
                "key" => &mut key,
                "ts" => &mut ts,
                "sig" => &mut sig,
                _ => return Err(AuthError::Malformed("an unknown parameter")),
            };
            if slot.replace(value).is_some() {
                return Err(AuthError::Malformed("a parameter twice"));
            }
        }
        let key = hex::<32>(key.ok_or(AuthError::Malformed("no key"))?)
            .and_then(|k| PublicKey::from_bytes(&k).ok())
            .ok_or(AuthError::Malformed("bad key"))?;
        let ts = ts
            .ok_or(AuthError::Malformed("no ts"))?
            .parse()
            .map_err(|_| AuthError::Malformed("bad ts"))?;
        let sig = hex::<64>(sig.ok_or(AuthError::Malformed("no sig"))?)
            .map(|s| Signature::from_bytes(&s))
            .ok_or(AuthError::Malformed("bad sig"))?;
        Ok(Auth { key, ts, sig })
    }

    /// Whether the request was signed by `key` at `ts`. The caller checks [`Auth::fresh`] and
    /// the replay cache.
    pub fn verify(&self, method: &str, path: &str, body: &[u8]) -> Result<(), AuthError> {
        self.key
            .verify(&message(method, path, self.ts, body), &self.sig)
            .map_err(|_| AuthError::BadSignature)
    }

    pub fn fresh(&self, now: u64) -> Result<(), AuthError> {
        if self.ts.abs_diff(now) <= MAX_SKEW {
            Ok(())
        } else {
            Err(AuthError::Stale)
        }
    }
}

/// What tells two players with the same nick apart: the key's first 4 bytes, as the lobby's
/// short keys.
pub fn fingerprint(key: &PublicKey) -> String {
    HEXLOWER.encode(&key.as_bytes()[..4])
}

/// The secret key as `/score key export` prints it and `import` takes it: plain hex, so that it
/// survives any copy and paste.
pub fn secret_to_text(secret: &SecretKey) -> String {
    HEXLOWER.encode(&secret.to_bytes())
}

pub fn secret_from_text(text: &str) -> Option<SecretKey> {
    hex::<32>(text.trim()).map(|b| SecretKey::from_bytes(&b))
}

fn hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    HEXLOWER_PERMISSIVE.decode(text.as_bytes()).ok()?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8032's test 1 secret key.
    const SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";

    const PINNED_MESSAGE: &str = "ctr-score/1\nPOST\n/runs\n1800000000\n\
        3348c5e89ab5656295f3c3994ea0ef2c663206f177d207fa350e2640f506f0c3";
    const PINNED_HEADER: &str = "CTR-Ed25519 \
        key=d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a, ts=1800000000, \
        sig=80bdf15b3c0edd8fcc9d9be730d319c68294410e24e6794bfa045838d47af362\
        790ad482c691d0ccad739cf3b7aad2935f2891609d9b59e5698d1f0626dd1f0c";

    #[test]
    fn rfc8032_test_1() {
        let secret = secret_from_text(SECRET).unwrap();
        assert_eq!(
            HEXLOWER.encode(secret.public().as_bytes()),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        assert_eq!(
            HEXLOWER.encode(&secret.sign(b"").to_bytes()),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
    }

    /// From `python3 crates/score/score_sign.py`: a change here breaks every deployed client or
    /// server.
    #[test]
    fn signs_as_pinned() {
        let secret = secret_from_text(SECRET).unwrap();
        let body = b"CTRR run bytes";
        assert_eq!(
            String::from_utf8(message("POST", "/runs", 1_800_000_000, body)).unwrap(),
            PINNED_MESSAGE
        );
        let auth = Auth::sign(&secret, "POST", "/runs", 1_800_000_000, body);
        assert_eq!(auth.header(), PINNED_HEADER);
        assert_eq!(Auth::parse(PINNED_HEADER), Ok(auth.clone()));
        assert_eq!(auth.verify("POST", "/runs", body), Ok(()));
        assert_eq!(auth.verify("POST", "/runs", b"other"), Err(AuthError::BadSignature));
        assert_eq!(auth.verify("DELETE", "/me", body), Err(AuthError::BadSignature));
        assert_eq!(auth.fresh(1_800_000_300), Ok(()));
        assert_eq!(auth.fresh(1_799_999_699), Err(AuthError::Stale));
    }

    #[test]
    fn refuses_malformed() {
        let ok = PINNED_HEADER;
        assert!(Auth::parse(&ok.replace("CTR-Ed25519", "Bearer")).is_err());
        assert!(Auth::parse(&ok.replace("ts=", "ts=x")).is_err());
        assert!(Auth::parse(&format!("{ok}, ts=1")).is_err());
        assert!(Auth::parse(&ok.replace(", sig=", ", nope=")).is_err());
    }

    #[test]
    fn secret_text_round_trips() {
        let secret = SecretKey::generate();
        let back = secret_from_text(&secret_to_text(&secret)).unwrap();
        assert_eq!(back.to_bytes(), secret.to_bytes());
        assert!(secret_from_text("abc").is_none());
    }
}
