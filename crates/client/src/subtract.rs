//! PS1 subtractive blending (background minus face), which Bevy's alpha modes
//! don't offer, as a blend-state override on `StandardMaterial`.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    SpecializedMeshPipelineError,
};

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct Subtract {}

impl MaterialExtension for Subtract {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let blend = BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::ReverseSubtract,
            },
            alpha: BlendComponent::OVER,
        };
        for target in descriptor
            .fragment
            .iter_mut()
            .flat_map(|f| f.targets.iter_mut())
            .flatten()
        {
            target.blend = Some(blend);
        }
        Ok(())
    }
}

pub type SubtractMaterial = ExtendedMaterial<StandardMaterial, Subtract>;
