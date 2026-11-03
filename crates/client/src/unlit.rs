//! Every material of ours is unlit, but Bevy's `StandardMaterial` draws through a fragment shader
//! with all of Bevy's lighting in it: `unlit` is a flag the shader reads as it runs, not one it is
//! compiled without. On WebGL2 the lights are a uniform array of 204 of them, which Firefox on
//! macOS (through Apple's OpenGL) takes about 8 s to link, for each of the mesh pipelines: most of
//! a minute before the first frame. So the lighting is cut out of Bevy's shader as it is loaded,
//! leaving what it does for an unlit material. A lit material, or a Bevy light, would be drawn
//! unlit.

use bevy::asset::AssetEventSystems;
use bevy::prelude::*;
use bevy::shader::{Shader, Source};

/// Where Bevy keeps the shader (`bevy_pbr`'s `load_shader_library!`).
const PATH: &str = "embedded://bevy_pbr/render/pbr.wesl";

/// The choice between the lit and the unlit color in Bevy's shader, and the unlit one alone.
const LIGHTING: &str =
    "    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
";
const UNLIT: &str = "    out.color = pbr_input.material.base_color;
";

pub struct UnlitPlugin;

impl Plugin for UnlitPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, check)
            .add_systems(PostUpdate, cut_lighting.after(AssetEventSystems));
    }
}

/// Tells if Bevy no longer has the shader where it is looked for (after an upgrade).
fn check(asset_server: Res<AssetServer>) {
    if asset_server.get_handle::<Shader>(PATH).is_none() {
        warn!("{PATH} is not loaded: the meshes are drawn with Bevy's lighting in their shader");
    }
}

/// Cuts the lighting in the frame the shader is loaded, before the render world takes it
/// (`extract_shaders` reads it as it is then). Untracked, as a change would make the render world
/// build its pipelines again. Left as it is if Bevy's shader no longer reads as [`LIGHTING`].
fn cut_lighting(
    mut events: MessageReader<AssetEvent<Shader>>,
    mut shaders: ResMut<Assets<Shader>>,
) {
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        let Some(shader) = shaders.get_mut_untracked(*id) else {
            continue;
        };
        if shader.path != PATH {
            continue;
        }
        let Source::Wesl(source) = &shader.source else {
            continue;
        };
        if source.matches(LIGHTING).count() != 1 {
            warn!("{PATH} has changed: the meshes are drawn with Bevy's lighting in their shader");
            continue;
        }
        shader.source = Source::Wesl(source.replace(LIGHTING, UNLIT).into());
    }
}
