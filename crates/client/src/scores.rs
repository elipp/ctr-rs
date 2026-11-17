//! Sending personal bests to the score server (TIME-TRIAL-RELIC-LEADERBOARDS.md, "Client"). Only
//! ever when the player asks, and only the stored best: the board keeps each player's best, so
//! any other run would be pointless to send. One submit at a time, as the server allows.

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
    sent: ToSubmit,
}

pub struct ScoresPlugin;

impl Plugin for ScoresPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Scores>()
            .add_systems(Update, (command, end_screen, answer).chain());
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
        return Err("A best is on its way to the board already".into());
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
    scores.sending = Some(Sending { answer, sent });
    Ok(())
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
    let Some(Sending { sent, .. }) = scores.sending.take() else {
        return;
    };
    // The end screen that sent it, if it is still up: no other can be Sending.
    let end = race_on
        .get_mut()
        .and_then(|race| race.solo_end.as_mut())
        .filter(|end| end.submit == Some(Submit::Sending));
    let response = answer.map_err(|e| e.to_string()).and_then(|a| a);
    let (submit, said) = match response {
        Ok(Response::Submitted(Submitted { run_id, rank, pb_ms, top_ms })) => {
            crate::pb::submitted(&sent, run_id);
            info!("submitted as run {run_id}: rank {rank}, best {pb_ms} ms, #1 {top_ms} ms");
            let said = if rank == 1 {
                "Your best is #1 on the board!".to_string()
            } else {
                format!("Your best is #{rank} on the board. #1: {}", crate::laps::clock(top_ms))
            };
            (Submit::Accepted { rank, top_ms }, Ok(said))
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
