//! Sending personal bests to the score server (TIME-TRIAL-RELIC-LEADERBOARDS.md, "Client"), and
//! `/score forget`. Only ever when the player asks, and only the stored best: the board keeps
//! each player's best, so any other run would be pointless to send. One signed request at a time
//! (the server takes one submit at a time).

use crate::console::console;

use bevy::prelude::*;
use race::solo::Kind;
use score::{Rejected, Response, Submitted};

use crate::command::Action;
use crate::pb::ToSubmit;
use crate::race_on::{RaceOn, Submit};
use crate::toast::Toast;

const RED: Color = Color::srgb(1.0, 0.4, 0.4);

#[derive(Resource, Default)]
struct Scores {
    sending: Option<Sending>,
}

struct Sending {
    answer: net::score::Answer,
    job: Job,
}

enum Job {
    Submit(ToSubmit),
    Forget,
}

pub struct ScoresPlugin;

impl Plugin for ScoresPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Scores>()
            .add_systems(Update, (command, forget, end_screen, answer).chain());
    }
}

/// Starts sending the stored best of `kind` over `laps` on track `id`; or says why not.
fn start(
    scores: &mut Scores,
    id: u32,
    kind: Kind,
    laps: u8,
    nick: &str,
) -> Result<(), String> {
    if scores.sending.is_some() {
        return Err("The board hasn't answered the last request yet".into());
    }
    let server = net::score::default_score().ok_or("No score server is set (CTR_SCORE)")?;
    let sent = crate::pb::to_submit(id, kind, laps)?;
    let key = crate::player::key()?;
    let answer = net::score::submit(
        server,
        key,
        nick.to_string(),
        crate::version::VERSION.to_string(),
        sent.final_ms,
        sent.bytes.clone(),
    );
    scores.sending = Some(Sending { answer, job: Job::Submit(sent) });
    Ok(())
}

/// `/score forget` tells what it does, and does it once given our key's fingerprint: a typo or a
/// line from the history can't wipe a player out.
fn forget(
    mut actions: MessageReader<Action>,
    mut scores: ResMut<Scores>,
    mut toasts: MessageWriter<Toast>,
) {
    for action in actions.read() {
        let Action::ScoreForget(confirm) = action else {
            continue;
        };
        let mut asked = || -> Result<Option<String>, String> {
            let key = crate::player::key()?;
            let fingerprint = score::fingerprint(&key.public());
            let Some(confirm) = confirm else {
                return Ok(Some(fingerprint));
            };
            if !confirm.eq_ignore_ascii_case(&fingerprint) {
                return Err(format!("You are {fingerprint}, not {confirm}"));
            }
            if scores.sending.is_some() {
                return Err("The board hasn't answered the last request yet".into());
            }
            let server = net::score::default_score().ok_or("No score server is set (CTR_SCORE)")?;
            let answer = net::score::forget(server, key);
            scores.sending = Some(Sending { answer, job: Job::Forget });
            Ok(None)
        };
        match asked() {
            Ok(Some(fingerprint)) => {
                console!(
                    "This deletes you ({fingerprint}) and all your runs from the boards, for \
                     good. Your bests stay here and can be submitted again. To go on: /score \
                     forget {fingerprint}"
                );
            }
            Ok(None) => {
                toasts.write(Toast::new("Asking the board to forget you", 2000));
            }
            Err(why) => {
                toasts.write(Toast::new(why, 3000).color(RED));
            }
        }
    }
}

fn command(
    mut actions: MessageReader<Action>,
    mut scores: ResMut<Scores>,
    config: Res<crate::config::Config>,
    level: Res<crate::resources::LevelNumber>,
    mut toasts: MessageWriter<Toast>,
) {
    for action in actions.read() {
        let Action::ScoreSubmit { kind, track } = action else {
            continue;
        };
        let id = track.unwrap_or(level.0);
        match start(&mut scores, id, *kind, race::DEFAULT_LAPS, &config.nickname) {
            Ok(()) => toasts.write(Toast::new("Sending your best to the board", 2000)),
            Err(why) => toasts.write(Toast::new(why, 3000).color(RED)),
        };
    }
}

fn end_screen(
    keys: Res<ButtonInput<KeyCode>>,
    pads: crate::drive::Pads,
    config: Res<crate::config::Config>,
    menu: Res<crate::pause::PauseMenu>,
    mut race_on: ResMut<RaceOn>,
    mut scores: ResMut<Scores>,
    mut toasts: MessageWriter<Toast>,
) {
    if menu.open || !crate::ghost::select_pressed(&keys, &pads, &config) {
        return;
    }
    let Some(race) = race_on.get_mut() else {
        return;
    };
    let (Some(kind), Some(end)) = (race.solo, race.solo_end.as_mut()) else {
        return;
    };
    if end.submit != Some(Submit::Offered) {
        return;
    }
    match start(&mut scores, race.track, kind, race.rules.laps, &config.nickname) {
        Ok(()) => end.submit = Some(Submit::Sending),
        Err(why) => {
            end.submit = Some(Submit::Refused);
            toasts.write(Toast::new(why, 3000).color(RED));
        }
    }
}

fn answer(
    mut scores: ResMut<Scores>,
    mut race_on: ResMut<RaceOn>,
    mut toasts: MessageWriter<Toast>,
) {
    let Some(answer) = scores.sending.as_ref().and_then(|s| s.answer.poll()) else {
        return;
    };
    let Some(Sending { job, .. }) = scores.sending.take() else {
        return;
    };
    let response = answer.map_err(|e| e.to_string()).and_then(|a| a);
    let sent = match job {
        Job::Submit(sent) => sent,
        Job::Forget => {
            forgotten(response, &mut toasts);
            return;
        }
    };
    // The end screen that sent it, if it is still up: no other can be Sending.
    let end = race_on
        .get_mut()
        .and_then(|race| race.solo_end.as_mut())
        .filter(|end| end.submit == Some(Submit::Sending));
    let (submit, said) = match response {
        Ok(Response::Submitted(Submitted { run_id, rank, pb_ms, top_ms })) => {
            crate::pb::submitted(&sent, run_id);
            info!("submitted as run {run_id}: rank {rank}, best {pb_ms} ms, #1 {top_ms} ms");
            (Submit::Accepted { rank, top_ms }, Ok(placed(rank, top_ms)))
        }
        Ok(Response::Rejected(rejected)) => {
            if matches!(rejected, Rejected::NoFinish | Rejected::Mismatch { .. }) {
                error!("the score server's replay of a {} ms run: {rejected}", sent.final_ms);
            }
            (Submit::Refused, Err(format!("The board turned it down: {rejected}")))
        }
        Ok(Response::Unauthorized { why, .. }) => {
            (Submit::Refused, Err(format!("The board turned the signature down: {why}")))
        }
        Ok(Response::TooMany) => (Submit::Refused, Err("Too many submits: try later".into())),
        Ok(Response::Busy) => (Submit::Refused, Err("The board is busy: try again".into())),
        Ok(Response::Failed) => (Submit::Refused, Err("The board failed to take it".into())),
        Ok(other) => {
            warn!("the score server answered a submit with {other:?}");
            (Submit::Refused, Err("The board answered nonsense".into()))
        }
        Err(e) => (Submit::Refused, Err(format!("Could not reach the board: {e}"))),
    };
    let shown = end.is_some();
    if let Some(end) = end {
        end.submit = Some(submit);
    }
    match said {
        // The end screen says it.
        Ok(_) if shown => {}
        Ok(said) => {
            toasts.write(Toast::new(said, 4000));
        }
        Err(why) => {
            toasts.write(Toast::new(why, 5000).color(RED));
        }
    }
}

/// Where the board put a submitted best, for both the end screen's line and the toast once that
/// screen is gone.
pub(crate) fn placed(rank: u32, top_ms: i32) -> String {
    // The server's ranks start at 1.
    let place = crate::lobby::ordinal((rank as usize).saturating_sub(1));
    match rank {
        1 => format!("{place} on the board!"),
        _ => format!("{place} on the board, top {}", race::clock::time(top_ms)),
    }
}

fn forgotten(response: Result<Response, String>, toasts: &mut MessageWriter<Toast>) {
    let why = match response {
        Ok(Response::Forgotten { runs }) => {
            crate::pb::forget_submitted();
            let runs = match runs {
                1 => "1 run".to_string(),
                n => format!("{n} runs"),
            };
            toasts.write(Toast::new(format!("The board forgot you, and {runs}"), 4000));
            return;
        }
        Ok(Response::Unauthorized { why, .. }) => {
            format!("The board turned the signature down: {why}")
        }
        Ok(Response::Failed) => "The board failed to forget you".into(),
        Ok(other) => {
            warn!("the score server answered a forget with {other:?}");
            "The board answered nonsense".into()
        }
        Err(e) => format!("Could not reach the board: {e}"),
    };
    toasts.write(Toast::new(why, 5000).color(RED));
}
