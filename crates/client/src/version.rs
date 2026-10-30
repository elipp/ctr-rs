//! `/version`: the build's version, as `build.rs` recorded it: the tag it comes after, and the
//! short sha of its commit unless that commit is the tag's.

use bevy::prelude::*;

use crate::command::Action;
use crate::toast::Toast;

/// The version of this build: `v0.1.0`, or `v0.1.0 (884cbee)` for a commit after the tag.
pub const VERSION: &str = env!("CTR_VERSION");

pub struct VersionPlugin;

impl Plugin for VersionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show);
    }
}

fn show(mut actions: MessageReader<Action>, mut toasts: MessageWriter<Toast>) {
    for action in actions.read() {
        if matches!(action, Action::Version) {
            info!("version {VERSION}");
            toasts.write(Toast::new(format!("Version {VERSION}"), 3000));
        }
    }
}
