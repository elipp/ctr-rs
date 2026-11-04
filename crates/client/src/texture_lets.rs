//! Bevy's PBR shaders put textures and samplers in `let`s, which WGSL does not allow. naga (and so
//! Chrome and Firefox) accepts it, but Safari rejects the shader and the game quits on the first
//! mesh. The `let`s are cut out as the shaders load and the bindings used directly instead.

// Built natively only for its tests.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use bevy::asset::AssetEventSystems;
use bevy::prelude::*;
use bevy::shader::{Shader, Source};

const PATHS: [&str; 3] = [
    "embedded://bevy_pbr/render/pbr_fragment.wesl",
    "embedded://bevy_pbr/render/pbr_prepass.wesl",
    "embedded://bevy_pbr/render/pbr_prepass_functions.wesl",
];

const BINDLESS: &str = "@if(BINDLESS)";

pub struct TextureLetsPlugin;

impl Plugin for TextureLetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, rewrite.after(AssetEventSystems));
    }
}

/// In the frame the shader is loaded, as `unlit::cut_lighting` does.
fn rewrite(mut events: MessageReader<AssetEvent<Shader>>, mut shaders: ResMut<Assets<Shader>>) {
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        let Some(shader) = shaders.get_mut_untracked(*id) else {
            continue;
        };
        if !PATHS.contains(&shader.path.as_str()) {
            continue;
        }
        let Source::Wesl(source) = &shader.source else {
            continue;
        };
        match inline(source) {
            Some(inlined) => shader.source = Source::Wesl(inlined.into()),
            None => warn!(
                "{} no longer has a texture in a `let`: `texture_lets` can go",
                shader.path
            ),
        }
    }
}

/// Cuts out each `@if(BINDLESS) let texture = ..; @else let texture = ..;` pair (and the same for
/// `sampler_`), and puts the non-bindless binding where the `let` was used: a browser has no
/// binding arrays, so is never bindless. None if there were no such pairs.
fn inline(source: &str) -> Option<String> {
    let mut out = String::with_capacity(source.len());
    let mut texture = None;
    let mut sampler = None;
    let mut rest = source;
    let mut changed = false;
    while let Some(at) = rest.find(BINDLESS) {
        let (before, pair) = rest.split_at(at);
        out.push_str(&use_bindings(before, texture, sampler));
        match parse_pair(pair) {
            Some((name, binding, after)) => {
                match name {
                    "texture" => texture = Some(binding),
                    _ => sampler = Some(binding),
                }
                rest = after;
                changed = true;
            }
            None => {
                out.push_str(BINDLESS);
                rest = &pair[BINDLESS.len()..];
            }
        }
    }
    out.push_str(&use_bindings(rest, texture, sampler));
    changed.then_some(out)
}

/// The `let`'s name, its non-bindless binding, and the text after the pair.
fn parse_pair(text: &str) -> Option<(&str, &str, &str)> {
    let text = text.strip_prefix(BINDLESS)?.trim_start();
    let text = text.strip_prefix("let ")?;
    let name = ["texture", "sampler_"]
        .into_iter()
        .find(|name| text.starts_with(&format!("{name} = ")))?;
    let text = &text[text.find(';')? + 1..];
    let text = text.trim_start().strip_prefix("@else")?.trim_start();
    let text = text.strip_prefix(&format!("let {name} = "))?;
    let end = text.find(';')?;
    Some((name, text[..end].trim(), &text[end + 1..]))
}

fn use_bindings(text: &str, texture: Option<&str>, sampler: Option<&str>) -> String {
    let text = match texture {
        Some(binding) => replace_word(text, "texture", binding),
        None => text.to_owned(),
    };
    match sampler {
        Some(binding) => replace_word(&text, "sampler_", binding),
        None => text,
    }
}

fn replace_word(text: &str, word: &str, with: &str) -> String {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(word) {
        let after = &rest[at + word.len()..];
        let is_word = !rest[..at].ends_with(is_ident) && !after.starts_with(is_ident);
        out.push_str(&rest[..at]);
        out.push_str(if is_word { with } else { word });
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_bindings_where_they_are_sampled() {
        let source = "    if x {
        @if(BINDLESS)
        let texture = bindless_textures_2d[
            material_indices[slot].base_color_texture
        ];
        @else
        let texture = pbr_bindings::base_color_texture;
@if(BINDLESS)
        let sampler_ = bindless_samplers_filtering[material_indices[slot].base_color_sampler];
@else
        let sampler_ = pbr_bindings::base_color_sampler;
        let c = textureSample(texture, sampler_, uv);
        let texture_size = 1;
    }
";
        let inlined = inline(source).unwrap();
        assert!(!inlined.contains("let texture ="));
        assert!(!inlined.contains("let sampler_ ="));
        assert!(!inlined.contains("bindless"));
        assert!(inlined.contains(
            "textureSample(pbr_bindings::base_color_texture, pbr_bindings::base_color_sampler, uv)"
        ));
        assert!(inlined.contains("let texture_size = 1;"));
    }

    #[test]
    fn leaves_other_bindless_choices() {
        let source = "@if(BINDLESS)\nlet m = a;\n@else\nlet m = b;\n";
        assert_eq!(inline(source), None);
    }
}
