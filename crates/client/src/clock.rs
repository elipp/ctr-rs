//! What a clock does to the screen (`DISPLAY_Blur_Main`, from `MainFrame_GameLogic`): its white
//! flash on everyone's, and then the trail on the screens of the drivers it slowed (and a faint
//! one on its user's). The strength of each tick is the sim's (`race::Race::last_blurs`).
//!
//! Retail draws the trail by taking the picture it showed last frame (the other buffer), a few
//! pixels in from each edge, stretched over the whole view and blended half over the new one, so
//! the old pictures zoom outward and fade. The inset pulses with `|sin(timer * 100)|`. It does
//! nothing on the first frame of a trail, which has no picture of its own before it.
//!
//! Here it is a pass over the picture of the kart's camera (the world and our kart, not the HUD),
//! after it is drawn, with the last frame's picture kept in a texture of its own. The per-frame
//! blend and zoom are scaled by how long the frame took, so the trail lasts as long as retail's
//! at 30 frames a second, whatever the frame rate. The blend is of the colours as stored
//! (sRGB), as the PS1's is.

use std::collections::HashMap;

use bevy::asset::embedded_asset;
use bevy::core_pipeline::schedule::Core3d;
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3dSystems, FullscreenShader};
use bevy::image::ToExtents;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, FragmentState, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType, Texture, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
    TextureViewDescriptor,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::sync_world::MainEntity;
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use crate::hud_view;
use crate::props::Sim;

/// Retail's frames a second, which the trail's blend and zoom are per.
const RETAIL_FPS: f32 = 30.0;
/// The size of retail's view with one player (`pushBuffer.rect`), which the insets are in.
const VIEW: Vec2 = Vec2::new(512.0, 216.0);
/// A frame without a tick for this long is a paused race (or a stalled one): retail draws no
/// trail when the game logic does not run.
const STALE_SECS: f64 = 0.1;

/// [`ClockTrail::mode`]: nothing drawn, and the kept picture is forgotten.
const OFF: u32 = 0;
/// The first frame of a trail: the picture is only kept.
const KEEP: u32 = 1;
/// The trail, and the picture kept.
const TRAIL: u32 = 2;
/// The white flash, kept as the picture the trail starts from.
const FLASH: u32 = 3;

/// The clock's pass over the picture of the camera that has it (the one that draws our kart over
/// the world), this frame.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default)]
#[extract_app(RenderApp)]
pub struct ClockTrail {
    mode: u32,
    /// How much of the kept picture is over the new one (retail's half, for each of its frames
    /// this one stands for).
    keep: f32,
    /// How far in from each edge the kept picture is taken, as a part of the picture.
    inset: Vec2,
}

pub struct ClockPlugin;

impl Plugin for ClockPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "clock.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<ClockTrail>::default(),
            UniformComponentPlugin::<ClockTrail>::default(),
        ))
        .add_systems(PostUpdate, update.run_if(crate::drive::driving));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<Histories>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_histories.in_set(RenderSystems::Prepare))
            .add_systems(
                Core3d,
                pass.in_set(Core3dSystems::PostProcess).before(tonemapping),
            );
    }
}

/// The strength of each view's screen this frame, and what it makes of the pass.
fn update(
    sim: Res<Sim>,
    drive: Res<crate::drive::Drive>,
    config: Res<crate::config::Config>,
    fixed: Res<Time<Fixed>>,
    real: Res<Time<Real>>,
    mut trails: Query<(&crate::drive::ViewOf, &mut ClockTrail)>,
    // The last tick seen, and when it was.
    mut seen: Local<Option<(u32, f64)>>,
    // Whether each view's pass drew last frame.
    mut drawn: Local<Vec<bool>>,
) {
    let now = real.elapsed_secs_f64();
    let tick = sim.race.tick;
    if seen.is_none_or(|(t, _)| t != tick) {
        *seen = Some((tick, now));
    }
    let stale = seen.is_some_and(|(_, at)| now - at > STALE_SECS);
    // Retail's frames this one stands for.
    let frames = (real.delta_secs() * RETAIL_FPS).clamp(0.0, 4.0);
    drawn.resize(hud_view::views(&sim, &drive), false);
    for (view, mut t) in &mut trails {
        let view = view.0;
        if view >= drawn.len() {
            continue;
        }
        // Shader effects off: no trail, so no pass (and no picture kept).
        let strength = if stale || !config.shader_effects {
            0
        } else {
            sim.race
                .last_blurs
                .get(hud_view::slot_of(&sim, &drive, view))
                .copied()
                .unwrap_or(0)
        };
        let trail = if strength < 0 {
            ClockTrail {
                mode: FLASH,
                ..default()
            }
        } else if strength == 0 {
            ClockTrail::default()
        } else if !drawn[view] {
            ClockTrail {
                mode: KEEP,
                ..default()
            }
        } else {
            // `timer + cameraID`, between the ticks, for the pulse.
            let wave = (tick as usize + view) as f32 + fixed.overstep_fraction();
            let sin = (wave * 100.0 / 4096.0 * std::f32::consts::TAU).sin().abs();
            let mut blur = sin * 1024.0 + 1024.0;
            if strength < 0x1000 {
                blur *= strength as f32 / 4096.0;
            }
            // The pixels in from each side, of retail's view.
            let inset = Vec2::new(blur * 9.0 / 4096.0 + 2.0, blur * 6.0 / 4096.0 + 2.0) / VIEW;
            // `frames` of retail's zooms and blends in one.
            let scale = (Vec2::ONE - 2.0 * inset).powf(frames);
            ClockTrail {
                mode: TRAIL,
                keep: 0.5f32.powf(frames),
                inset: (Vec2::ONE - scale) / 2.0,
            }
        };
        drawn[view] = trail.mode != OFF;
        *t = trail;
    }
}

#[derive(Resource)]
struct ClockPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    shader: Handle<Shader>,
    vertex: FullscreenShader,
    /// By the format of the picture.
    pipelines: HashMap<TextureFormat, CachedRenderPipelineId>,
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "clock_trail_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                uniform_buffer::<ClockTrail>(true),
            ),
        ),
    );
    commands.insert_resource(ClockPipeline {
        layout,
        sampler: render_device.create_sampler(&SamplerDescriptor {
            mag_filter: bevy::render::render_resource::FilterMode::Linear,
            min_filter: bevy::render::render_resource::FilterMode::Linear,
            ..default()
        }),
        shader: asset_server.load(concat!("embedded://", env!("CARGO_PKG_NAME"), "/clock.wgsl")),
        vertex: fullscreen_shader.clone(),
        pipelines: HashMap::new(),
    });
}

/// The picture each camera showed last frame, by its entity in the main world.
#[derive(Resource, Default)]
struct Histories(HashMap<MainEntity, History>);

struct History {
    texture: Texture,
    view: TextureView,
    size: UVec2,
    format: TextureFormat,
}

/// A kept picture of the camera's size and format for each camera with the pass, and its
/// pipeline; the others' go.
fn prepare_histories(
    mut histories: ResMut<Histories>,
    mut pipeline: ResMut<ClockPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    views: Query<(&MainEntity, &ExtractedCamera, &ViewTarget, &ClockTrail)>,
) {
    let mut wanted = Vec::new();
    for (main, camera, target, _) in &views {
        let Some(size) = camera.physical_target_size else {
            continue;
        };
        let format = target.main_texture_format();
        wanted.push(*main);
        let ClockPipeline {
            layout,
            shader,
            vertex,
            pipelines,
            ..
        } = &mut *pipeline;
        pipelines.entry(format).or_insert_with(|| {
            pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("clock_trail_pipeline".into()),
                layout: vec![layout.clone()],
                vertex: vertex.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: shader.clone(),
                    targets: vec![Some(ColorTargetState {
                        format,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                ..default()
            })
        });
        if histories
            .0
            .get(main)
            .is_some_and(|h| h.size == size && h.format == format)
        {
            continue;
        }
        let texture = render_device.create_texture(&TextureDescriptor {
            label: Some("clock_trail_history"),
            size: size.to_extents(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        histories.0.insert(
            *main,
            History {
                texture,
                view,
                size,
                format,
            },
        );
    }
    histories.0.retain(|main, _| wanted.contains(main));
}

/// Draws the trail or the flash over the picture, and keeps what is shown.
fn pass(
    view: ViewQuery<(
        &MainEntity,
        &ViewTarget,
        &ClockTrail,
        &DynamicUniformIndex<ClockTrail>,
    )>,
    histories: Res<Histories>,
    pipeline: Res<ClockPipeline>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<ClockTrail>>,
    render_device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (main, target, trail, index) = view.into_inner();
    let Some(history) = histories.0.get(main) else {
        return;
    };
    if trail.mode == OFF {
        return;
    }
    if trail.mode != KEEP {
        let Some(render_pipeline) = pipeline
            .pipelines
            .get(&target.main_texture_format())
            .and_then(|&id| pipeline_cache.get_render_pipeline(id))
        else {
            return;
        };
        let Some(binding) = uniforms.uniforms().binding() else {
            return;
        };
        let post_process = target.post_process_write();
        let bind_group = render_device.create_bind_group(
            "clock_trail_bind_group",
            &pipeline_cache.get_bind_group_layout(&pipeline.layout),
            &BindGroupEntries::sequential((
                post_process.source,
                &pipeline.sampler,
                &history.view,
                binding,
            )),
        );
        let mut render_pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("clock_trail_pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: post_process.destination,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations::default(),
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        render_pass.set_pipeline(render_pipeline);
        render_pass.set_bind_group(0, &bind_group, &[index.index()]);
        render_pass.draw(0..3, 0..1);
    }
    // What is shown now is what the next frame's trail is taken from.
    ctx.command_encoder().copy_texture_to_texture(
        target.main_texture().as_image_copy(),
        history.texture.as_image_copy(),
        history.size.to_extents(),
    );
}
