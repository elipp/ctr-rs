//! The heat shimmer of the flame jets of Tiger Temple (`emSet_fjHeat`, `Torch_Main`), and the
//! warpball's glow (`emSet_Warpball`), which is a lens of the same kind. Retail draws it for one
//! player only, as it costs the PS1 a pass over the picture; we do it always.
//!
//! A heat particle is not a sprite but a lens: three octagons round it (the radii of its rings
//! grow as it ages), and the triangles between them, which `Torch_Main` draws showing the
//! picture from other places than they cover, so that the air wobbles. The lenses go to the
//! picture's post-processing ([`HeatWarp`], `heat.wesl`) as the points of their octagons on the
//! screen, with the triangles of each sector worked out there; the camera's picture is what the
//! game's framebuffer is to retail. How the particles are born, move and grow is `render`'s and
//! [`crate::fire`]'s.
//!
//! Retail draws a lens into its ordering table at its own depth, so it warps only what was drawn
//! before it, what is behind it: the warpball's is a slot behind the ball (`otIndexOffset =
//! depthBiasNormal + 1`), so the ball is drawn over it, not warped, and a flame jet's fire is a
//! slot in front of its heat (`depthBiasNormal - 1`). We warp the picture once, between the
//! opaque and the see-through things (the ball and the fire are see-through), and the pass reads
//! the depth buffer to leave alone the opaque things nearer than a lens; where a lens would show
//! the picture from a place nearer than itself, it leaves that pixel as it is. That is a
//! look-alike, not retail's way: see-through things behind a lens are not warped.

use std::collections::HashMap;

use bevy::asset::embedded_asset;
use bevy::core_pipeline::core_3d::{main_opaque_pass_3d, main_transparent_pass_3d};
use bevy::core_pipeline::schedule::Core3d;
use bevy::core_pipeline::{Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, FragmentState, MultisampleState, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType, TextureFormat,
    TextureSampleType, TextureUsages,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::{ViewDepthStencilTexture, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderDefVal;
use render::mesh::SCALE;

use crate::exhaust::SCREEN_PIXEL_ASPECT;
use crate::fire::Fire;
use crate::props::Sim;

/// How many lenses retail draws a frame: the newest particles, 12 of them (`Torch_Main`).
const LENSES: usize = 12;
/// What a lens takes in the uniform, in `Vec4`s: its box, its centre, and 8 points a ring.
const PER_LENS: usize = 2 + 3 * 8;
/// The height of a ring as a part of its width (`CTC2(0x0a00, 2)`: the picture's pixels are
/// taller than wide).
const RING_HEIGHT: f32 = 0xa00 as f32 / 4096.0;
/// The corners of a ring, as a part of its radius (`0xb50 / 0x1000`).
const DIAGONAL: f32 = 0xb50 as f32 / 4096.0;
/// A lens whose outer ring is wider than this, as a part of half the picture, is not drawn
/// (`sxy0 < 0x80` of 0x100 pixels).
const MAX_WIDTH: f32 = 0.5;
/// How much further than retail a flame jet's lens takes the picture from. This is an
/// intentional departure from retail (whose strength is 1): at its own strength, the flame jets'
/// shimmer is hard to see. It stays under 3, where their rings (r0 - 6, r0 - 4, r0) would make
/// the warp fold over on itself.
const FLAME_STRENGTH: f32 = 2.9;
/// How much bigger than retail's a flame jet's lens is, its rings all grown alike (so that they
/// still fold only over 3). This is an intentional departure from retail (whose size is 1), so
/// that the shimmer has more impact.
const FLAME_SIZE: (i32, i32) = (5, 4);
/// The warpball's glow takes the picture from as far as retail does. Its rings are spaced so
/// that anything over 2 folds the warp (at any [`GLOW_SIZE`]).
const GLOW_STRENGTH: f32 = 1.0;
/// How much bigger than retail's the warpball's glow is, its rings all grown alike. This is an
/// intentional departure from retail (whose size is 1), so that the glow has more impact. A
/// bigger lens stops being drawn sooner as the ball comes near (see [`MAX_WIDTH`]).
const GLOW_SIZE: (i32, i32) = (3, 2);
/// How far behind the ball its glow is, in game units: one slot of retail's ordering table
/// (`viewZ >> 6`), as `RB_Warpball_ThTick` puts it there (`depthBiasNormal + 1`).
const GLOW_BEHIND: f32 = 64.0;

/// The lenses of the frame, for the post-processing of the main camera.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType)]
#[extract_app(RenderApp)]
pub struct HeatWarp {
    count: u32,
    /// For each lens: the box round it in the picture (min x, min y, max x, max y, as 0 to 1),
    /// its centre (with its strength in `z` and its depth, as the depth buffer has it, in `w`),
    /// and the points of its rings, outermost first, from the top clockwise.
    points: [Vec4; LENSES * PER_LENS],
}

impl Default for HeatWarp {
    fn default() -> HeatWarp {
        HeatWarp {
            count: 0,
            points: [Vec4::ZERO; LENSES * PER_LENS],
        }
    }
}

pub struct HeatPlugin;

impl Plugin for HeatPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "heat.wesl");
        app.add_plugins((
            ExtractComponentPlugin::<HeatWarp>::default(),
            UniformComponentPlugin::<HeatWarp>::default(),
        ))
        .add_observer(readable_depth)
        // After the camera moved this frame (the lenses go where it sees them now).
        .add_systems(PostUpdate, update.before(TransformSystems::Propagate));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_pipelines.in_set(RenderSystems::Prepare))
            .add_systems(
                Core3d,
                // Over the opaque things, under the see-through ones.
                pass.in_set(Core3dSystems::MainPass)
                    .after(main_opaque_pass_3d)
                    .before(main_transparent_pass_3d),
            );
    }
}

/// The pass reads the depth buffer of the camera that has it.
fn readable_depth(add: On<Add<HeatWarp>>, mut cameras: Query<&mut Camera3d>) {
    if let Ok(mut camera) = cameras.get_mut(add.entity) {
        let usages = TextureUsages::from(camera.depth_texture_usages);
        camera.depth_texture_usages = (usages | TextureUsages::TEXTURE_BINDING).into();
    }
}

/// The warpball's glow (`emSet_Warpball`, set each tick at the end of `RB_Warpball_ThTick`): its
/// height over the ball and the size of its rings both come from the last of
/// `R231.warpballParticleHeight`, which is the one retail always takes.
const GLOW_HEIGHT: i32 = 255;
/// The radii of the glow's rings: `height << 8`, `height * 0xc0` and `height << 7` in the axes,
/// in game units (as [`render::particles::Particle::heat_radii`] reads them), grown by
/// [`GLOW_SIZE`].
const GLOW_RADII: [i32; 3] = [
    ((GLOW_HEIGHT << 8) >> 8 & 0xff) * GLOW_SIZE.0 / GLOW_SIZE.1,
    ((GLOW_HEIGHT * 0xc0) >> 8 & 0xff) * GLOW_SIZE.0 / GLOW_SIZE.1,
    ((GLOW_HEIGHT << 7) >> 8 & 0xff) * GLOW_SIZE.0 / GLOW_SIZE.1,
];

/// Puts the lenses where the camera sees them, between the last two ticks.
fn update(
    fire: Option<Res<Fire>>,
    sim: Option<Res<Sim>>,
    config: Res<crate::config::Config>,
    fixed: Res<Time<Fixed>>,
    mut cameras: Query<(&Camera, &Transform, &mut HeatWarp), With<crate::drive::ChaseCamera>>,
    balls: Query<(&crate::projectile::Body, &Transform), Without<HeatWarp>>,
) {
    let Ok((camera, transform, mut warp)) = cameras.single_mut() else {
        return;
    };
    // With no lenses, the pass is not drawn.
    if !config.shader_effects {
        warp.count = 0;
        return;
    }
    let mut lenses: Vec<(f32, [Vec4; PER_LENS])> = Vec::new();
    let view = View {
        camera,
        eye: GlobalTransform::from(*transform),
        right: transform.rotation * Vec3::X,
        up: transform.rotation * Vec3::Y,
    };
    let forward = transform.rotation * Vec3::NEG_Z;
    if let Some(fire) = fire {
        let t = fixed.overstep_fraction();
        // The newest first, as retail's list is, and it looks at no more than 12 (it counts
        // the ones that are not seen too).
        for spark in fire.heat.iter().rev().take(LENSES) {
            let age = spark.lead + t;
            if spark.ticks as f32 + age >= spark.life {
                continue;
            }
            let p = spark.particle.ahead(age);
            let center = Vec3::from_array(p.position().map(|c| c as f32)) / 256.0 * SCALE;
            // At its own depth (`otIndexOffset = depthBiasNormal`, of the flame jet).
            let radii = p.heat_radii().map(|r| r * FLAME_SIZE.0 / FLAME_SIZE.1);
            if let Some(lens) = lens(&view, center, center, radii, FLAME_STRENGTH) {
                lenses.push(((center - transform.translation).dot(forward), lens));
            }
        }
    }
    // The warpball's glow, while it flies (`RB_Warpball_Death` ends the particle), over the ball
    // where it is drawn. Its particle is older than any flame's, so it comes after them in the
    // list, if there is room.
    let flying = sim
        .iter()
        .flat_map(|sim| sim.race.weapons.warpballs())
        .filter(|w| !w.fading);
    for w in flying {
        if lenses.len() >= LENSES {
            break;
        }
        let Some((_, ball)) = balls.iter().find(|(body, _)| body.id() == w.id) else {
            continue;
        };
        let center = ball.translation + Vec3::Y * (GLOW_HEIGHT as f32 * SCALE);
        // A slot behind the ball, so that the ball is not warped.
        let behind = ball.translation + forward * (GLOW_BEHIND * SCALE);
        if let Some(lens) = lens(&view, center, behind, GLOW_RADII, GLOW_STRENGTH) {
            lenses.push(((center - transform.translation).dot(forward), lens));
        }
    }
    // Drawn from the farthest to the nearest.
    lenses.sort_by(|a, b| b.0.total_cmp(&a.0));
    warp.count = lenses.len() as u32;
    for (n, (_, lens)) in lenses.iter().enumerate() {
        warp.points[n * PER_LENS..][..PER_LENS].copy_from_slice(lens);
    }
}

/// The camera the lenses are put where it sees them.
struct View<'a> {
    camera: &'a Camera,
    eye: GlobalTransform,
    right: Vec3,
    up: Vec3,
}

/// The points of a lens at `center` with rings of the `radii` (game units) as the picture's
/// coordinates, its `strength` (see [`FLAME_STRENGTH`]) and the depth of `behind`, the place it
/// is in retail's ordering table, or none if it is not in view or is too close to be drawn.
fn lens(
    view: &View,
    center: Vec3,
    behind: Vec3,
    radii: [i32; 3],
    strength: f32,
) -> Option<[Vec4; PER_LENS]> {
    let to_ndc = |at: Vec3| -> Option<Vec3> {
        let ndc = view.camera.world_to_ndc(&view.eye, at)?;
        (ndc.z > 0.0).then_some(ndc)
    };
    let to_picture = |at: Vec3| -> Option<Vec2> {
        let ndc = to_ndc(at)?;
        Some(Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5))
    };
    let middle = to_picture(center)?;
    // Farther than the centre, so in view if it is (the depth buffer's is 0 out at infinity).
    let depth = to_ndc(behind).map_or(0.0, |ndc| ndc.z);
    let mut points = Vec::with_capacity(PER_LENS);
    points.push(Vec4::ZERO);
    points.push(middle.extend(strength).extend(depth));
    for (ring, radius) in radii.into_iter().enumerate() {
        let across = view.right * (radius as f32 * SCALE);
        let high = view.up * (radius as f32 * SCALE * RING_HEIGHT * SCREEN_PIXEL_ASPECT);
        let d = DIAGONAL;
        for offset in [
            high,
            across * d + high * d,
            across,
            across * d - high * d,
            -high,
            -across * d - high * d,
            -across,
            -across * d + high * d,
        ] {
            let point = to_picture(center + offset)?;
            if ring == 0 && offset == across && (point.x - middle.x).abs() >= MAX_WIDTH {
                return None;
            }
            points.push(point.extend(0.0).extend(0.0));
        }
    }
    let (mut low, mut high) = (Vec2::MAX, Vec2::MIN);
    for p in &points[1..] {
        low = low.min(Vec2::new(p.x, p.y));
        high = high.max(Vec2::new(p.x, p.y));
    }
    points[0] = Vec4::new(low.x, low.y, high.x, high.y);
    points.try_into().ok()
}

#[derive(Resource)]
struct HeatPipeline {
    /// With the depth buffer as it is without multisampling, and as it is with.
    layouts: [BindGroupLayoutDescriptor; 2],
    sampler: Sampler,
    shader: Handle<Shader>,
    vertex: FullscreenShader,
    /// By the format of the picture and its samples.
    pipelines: HashMap<(TextureFormat, u32), CachedRenderPipelineId>,
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
) {
    let layout = |label: &'static str, depth| {
        BindGroupLayoutDescriptor::new(
            label,
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<HeatWarp>(true),
                    depth,
                ),
            ),
        )
    };
    commands.insert_resource(HeatPipeline {
        layouts: [
            // As an unfilterable float texture, not a depth one: WebGL2's GLSL cannot
            // `textureLoad` a depth texture, but can a float one (the same depth, in `r`).
            layout(
                "heat_warp_bind_group_layout",
                texture_2d(TextureSampleType::Float { filterable: false }),
            ),
            layout(
                "heat_warp_multisampled_bind_group_layout",
                texture_depth_2d_multisampled(),
            ),
        ],
        sampler: render_device.create_sampler(&SamplerDescriptor::default()),
        shader: asset_server.load(concat!("embedded://", env!("CARGO_PKG_NAME"), "/heat.wesl")),
        vertex: fullscreen_shader.clone(),
        pipelines: HashMap::new(),
    });
}

/// The pipeline each camera with the pass wants.
fn prepare_pipelines(
    mut pipeline: ResMut<HeatPipeline>,
    pipeline_cache: Res<PipelineCache>,
    views: Query<(&ViewTarget, &Msaa), With<HeatWarp>>,
) {
    for (target, msaa) in &views {
        let format = target.main_texture_format();
        let samples = msaa.samples();
        let multisampled = samples > 1;
        let HeatPipeline {
            layouts,
            shader,
            vertex,
            pipelines,
            ..
        } = &mut *pipeline;
        pipelines.entry((format, samples)).or_insert_with(|| {
            pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("heat_warp_pipeline".into()),
                layout: vec![layouts[multisampled as usize].clone()],
                vertex: vertex.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: shader.clone(),
                    // Given either way, for the shader's `@if(!MULTISAMPLED)`.
                    shader_defs: vec![ShaderDefVal::Bool("MULTISAMPLED".into(), multisampled)],
                    targets: vec![Some(ColorTargetState {
                        format,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                // Into the multisampled picture, if it is: the see-through things are drawn
                // into that after.
                multisample: MultisampleState {
                    count: samples,
                    ..default()
                },
                ..default()
            })
        });
    }
}

/// Warps the picture through the frame's lenses, if there are any.
fn pass(
    view: ViewQuery<(
        &ViewTarget,
        &ViewDepthStencilTexture,
        &Msaa,
        &HeatWarp,
        &DynamicUniformIndex<HeatWarp>,
    )>,
    pipeline: Res<HeatPipeline>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<HeatWarp>>,
    render_device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (target, depth, msaa, warp, index) = view.into_inner();
    if warp.count == 0 {
        return;
    }
    let multisampled = msaa.samples() > 1;
    let Some(render_pipeline) = pipeline
        .pipelines
        .get(&(target.main_texture_format(), msaa.samples()))
        .and_then(|&id| pipeline_cache.get_render_pipeline(id))
    else {
        return;
    };
    let Some(binding) = uniforms.uniforms().binding() else {
        return;
    };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else {
        return;
    };
    // From the opaque things' picture, resolved at the end of their pass. With multisampling,
    // the see-through things are drawn into the multisampled picture and that resolved over the
    // main one, so the warp is drawn into it too (each sample the same), and resolved.
    let post_process = target.post_process_write();
    let (view, resolve_target) = match target.sampled_main_texture_view() {
        Some(sampled) if multisampled => (sampled, Some(&**post_process.destination)),
        _ => (post_process.destination, None),
    };
    let bind_group = render_device.create_bind_group(
        "heat_warp_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipeline.layouts[multisampled as usize]),
        &BindGroupEntries::sequential((
            post_process.source,
            &pipeline.sampler,
            binding,
            depth_view,
        )),
    );
    let mut render_pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("heat_warp_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target,
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
