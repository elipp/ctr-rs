//! The operator's commands, run beside the server on the same database (WAL lets both write):
//! `docker compose exec score score-server <command>` (the image's working dir has `scores.db`).
//! A key is given as its hex, or a prefix of it (a fingerprint) that only one player or ban has.

use data_encoding::{HEXLOWER, HEXLOWER_PERMISSIVE};
use score::Rejected;

use crate::db::Db;

pub fn run(db: &Db, command: &[String], now: i64) -> Result<(), String> {
    let words: Vec<&str> = command.iter().map(String::as_str).collect();
    let e = |e: rusqlite::Error| e.to_string();
    match words.as_slice() {
        ["stats"] => stats(db).map_err(e),
        ["suspects"] => suspects(db).map_err(e),
        ["ban", key] => {
            let key = resolve(db, key)?;
            let who = who(db, &key)?;
            if db.ban(&key, now).map_err(e)? {
                println!("banned {who}: their runs are off the boards, and their submits refused");
            } else {
                println!("{who} was banned already");
            }
            Ok(())
        }
        ["unban", key] => {
            let key = resolve(db, key)?;
            let who = who(db, &key)?;
            if db.unban(&key).map_err(e)? {
                println!("unbanned {who}: their runs are back on the boards");
            } else {
                println!("{who} wasn't banned");
            }
            Ok(())
        }
        ["delete", id] => {
            let id: i64 = id.parse().map_err(|_| format!("'{id}' is not a run id"))?;
            if db.delete(id).map_err(e)? {
                println!("deleted run {id}");
            } else {
                println!("there is no run {id}");
            }
            Ok(())
        }
        _ => Err(format!("unknown command '{}'", words.join(" "))),
    }
}

/// The one key that `text` (hex, whole or the start of it) names.
fn resolve(db: &Db, text: &str) -> Result<[u8; 32], String> {
    let bytes = HEXLOWER_PERMISSIVE
        .decode(text.as_bytes())
        .map_err(|_| format!("'{text}' is not a key, nor an even number of hex digits of one"))?;
    if let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) {
        return Ok(key);
    }
    if bytes.len() < 4 || bytes.len() > 32 {
        return Err(format!("'{text}': give at least a fingerprint (8 hex digits) of a key"));
    }
    let keys = db.keys_starting(&bytes).map_err(|e| e.to_string())?;
    match keys.as_slice() {
        [key] => Ok(*key),
        [] => Err(format!("no player or ban has a key starting with {text}")),
        _ => Err(format!(
            "more than one key starts with {text}: {}",
            keys.iter().map(|k| HEXLOWER.encode(k)).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn fingerprint(key: &[u8; 32]) -> String {
    HEXLOWER.encode(&key[..4])
}

fn who(db: &Db, key: &[u8; 32]) -> Result<String, String> {
    let nick = db.nick(key).map_err(|e| e.to_string())?;
    Ok(match nick.as_deref() {
        Some("") | None => fingerprint(key),
        Some(nick) => format!("{nick} ({})", fingerprint(key)),
    })
}

fn date(at: i64) -> String {
    jiff::Timestamp::from_second(at)
        .map_or_else(|_| at.to_string(), |t| t.strftime("%Y-%m-%d %H:%M UTC").to_string())
}

fn stats(db: &Db) -> rusqlite::Result<()> {
    let stats = db.stats()?;
    println!(
        "{} players ({} banned), {} runs",
        stats.players, stats.banned, stats.runs
    );
    for (sim, boards, runs) in &stats.per_sim {
        println!("  sim {sim}: {boards} boards, {runs} runs");
    }
    if stats.rejections.is_empty() {
        return Ok(());
    }
    // `no finish` and `mismatch` are the client's sim and ours disagreeing: a bug, or a
    // forgotten SIM_VERSION bump in that build.
    println!("rejections by client version:");
    let mut version: Option<&str> = None;
    for (client, code, n) in &stats.rejections {
        if version != Some(client.as_str()) {
            println!("  {}:", if client.is_empty() { "?" } else { client });
            version = Some(client);
        }
        println!("    {}: {n}", Rejected::code_name(*code));
    }
    Ok(())
}

fn suspects(db: &Db) -> rusqlite::Result<()> {
    let suspects = db.suspects()?;
    if suspects.is_empty() {
        println!("no suspect runs");
    }
    for s in suspects {
        let name = |nick: &str, key: &[u8; 32]| match nick {
            "" => fingerprint(key),
            nick => format!("{nick} ({})", fingerprint(key)),
        };
        println!(
            "run {} by {}, {} at {}\n  much like run {} by {}, {} at {}",
            s.id,
            name(&s.nick, &s.key),
            race::clock::time(s.final_ms),
            date(s.at),
            s.of_id,
            name(&s.of_nick, &s.of_key),
            race::clock::time(s.of_final_ms),
            date(s.of_at),
        );
    }
    Ok(())
}
