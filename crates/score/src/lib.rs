//! What the leaderboard client and `score-server` must agree on (TIME-TRIAL-RELIC-LEADERBOARDS.md):
//! the protocol, how a write is signed by the player key, and how that key is shown and moved.
//! One copy for both, so they can't drift; the test pins the signed bytes against an independent
//! implementation (`score_sign.py`).
//!
//! The key types are iroh's (`net::SecretKey` is the same type), which sign and generate on
//! native and wasm alike. The player key is never an endpoint key, though.

mod proto;

pub use proto::*;

use data_encoding::{HEXLOWER, HEXLOWER_PERMISSIVE};
pub use iroh_base::{PublicKey, SecretKey, Signature};
use sha2::{Digest, Sha256};

/// Leads the signed bytes: a signature for anything else can't be passed off as a request's.
const DOMAIN: &str = "ctr-score/1";
/// How far a request's `ts` may be from the server's clock, either way.
pub const MAX_SKEW: u64 = 300;

/// What a signature is for. Spelled as HTTP requests because the signed bytes were fixed first
/// for HTTPS: `score_sign.py` and the pinned test still check them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Submit,
    Forget,
}

impl Action {
    fn spelled(self) -> &'static str {
        match self {
            Action::Submit => "POST\n/runs",
            Action::Forget => "DELETE\n/me",
        }
    }
}

/// What a signature is over.
pub fn message(action: Action, ts: u64, body: &[u8]) -> Vec<u8> {
    let digest = HEXLOWER.encode(&Sha256::digest(body));
    format!("{DOMAIN}\n{}\n{ts}\n{digest}", action.spelled()).into_bytes()
}

/// A request signed by a player key: the key is the player.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct Auth {
    pub key: [u8; 32],
    /// Unix seconds, by the client's clock (plus the offset an `Unauthorized` told it).
    pub ts: u64,
    pub sig: [u8; 64],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    BadKey,
    BadSignature,
    Stale,
    Replayed,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AuthError::BadKey => "not a public key",
            AuthError::BadSignature => "bad signature",
            AuthError::Stale => "the clock is off",
            AuthError::Replayed => "signature used before",
        })
    }
}

impl Auth {
    pub fn sign(secret: &SecretKey, action: Action, ts: u64, body: &[u8]) -> Auth {
        Auth {
            key: *secret.public().as_bytes(),
            ts,
            sig: secret.sign(&message(action, ts, body)).to_bytes(),
        }
    }

    /// The key that signed `body` for `action` at `ts`. The caller checks [`Auth::fresh`] and
    /// the replay cache.
    pub fn verify(&self, action: Action, body: &[u8]) -> Result<PublicKey, AuthError> {
        let key = PublicKey::from_bytes(&self.key).map_err(|_| AuthError::BadKey)?;
        key.verify(&message(action, self.ts, body), &Signature::from_bytes(&self.sig))
            .map_err(|_| AuthError::BadSignature)?;
        Ok(key)
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

/// The most characters of a nickname.
pub const NICK_MAX: usize = 12;

/// The letters a nickname may have besides printable ASCII: the Latin script of Europe's
/// languages. Other scripts are left out: Greek and Cyrillic have letters that look like Latin
/// ones, which would let a nick pass for another.
pub const NICK_EXTRA: &str = concat!(
    // French, Spanish, Portuguese, Italian, Catalan, German, Dutch
    "áÁàÀâÂäÄãÃåÅæÆçÇéÉèÈêÊëËíÍìÌîÎïÏñÑóÓòÒôÔöÖõÕúÚùÙûÛüÜýÝÿŸœŒß",
    // Nordic, Icelandic, Welsh
    "øØðÐþÞŵŴŷŶ",
    // Polish, Czech, Slovak, Hungarian, Croatian, Slovene
    "ąĄćĆęĘłŁńŃśŚźŹżŻčČďĎěĚňŇřŘšŠťŤůŮžŽĺĹľĽŕŔőŐűŰđĐ",
    // Latvian, Lithuanian (Estonian's are above)
    "āĀēĒīĪōŌūŪģĢķĶļĻņŅėĖįĮųŲ",
    // Romanian, Turkish, Maltese
    "ăĂșȘțȚşŞğĞıİċĊġĠħĦ",
);

pub fn nick_char(c: char) -> bool {
    c.is_ascii_graphic() || c == ' ' || NICK_EXTRA.contains(c)
}

/// A nickname as it is kept and sent: only [`nick_char`]s, trimmed, and at most [`NICK_MAX`]
/// characters. Empty means none.
pub fn clean_nick(nick: &str) -> String {
    nick.chars()
        .filter(|&c| nick_char(c))
        .collect::<String>()
        .trim()
        .chars()
        .take(NICK_MAX)
        .collect()
}

fn hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    HEXLOWER_PERMISSIVE
        .decode(text.as_bytes())
        .ok()?
        .try_into()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8032's test 1 secret key.
    const SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";

    const PINNED_MESSAGE: &str = "ctr-score/1\nPOST\n/runs\n1800000000\n\
        3348c5e89ab5656295f3c3994ea0ef2c663206f177d207fa350e2640f506f0c3";
    const PINNED_KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const PINNED_SIG: &str = "80bdf15b3c0edd8fcc9d9be730d319c68294410e24e6794bfa045838d47af362\
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
            String::from_utf8(message(Action::Submit, 1_800_000_000, body)).unwrap(),
            PINNED_MESSAGE
        );
        let auth = Auth::sign(&secret, Action::Submit, 1_800_000_000, body);
        assert_eq!(HEXLOWER.encode(&auth.key), PINNED_KEY);
        assert_eq!(HEXLOWER.encode(&auth.sig), PINNED_SIG);
        assert_eq!(auth.verify(Action::Submit, body), Ok(secret.public()));
        assert_eq!(auth.verify(Action::Submit, b"other"), Err(AuthError::BadSignature));
        assert_eq!(auth.verify(Action::Forget, body), Err(AuthError::BadSignature));
        assert_eq!(auth.fresh(1_800_000_300), Ok(()));
        assert_eq!(auth.fresh(1_799_999_699), Err(AuthError::Stale));
    }

    #[test]
    fn nick_keeps_only_its_letters() {
        assert_eq!(clean_nick(" Ñandú\tŒuvre "), "ÑandúŒuvre");
        assert_eq!(clean_nick("a€b✨c\u{7}d"), "abcd");
        assert_eq!(clean_nick("日本"), "");
        assert_eq!(clean_nick("Straße Łódź"), "Straße Łódź");
        assert_eq!(clean_nick("Āžuoliņš"), "Āžuoliņš");
        assert_eq!(clean_nick("ªƒºΑ"), "");
    }

    #[test]
    fn secret_text_round_trips() {
        let secret = SecretKey::generate();
        let back = secret_from_text(&secret_to_text(&secret)).unwrap();
        assert_eq!(back.to_bytes(), secret.to_bytes());
        assert!(secret_from_text("abc").is_none());
    }
}
