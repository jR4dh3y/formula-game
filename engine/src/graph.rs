use crate::memory::{Arena, Handle};
use std::collections::HashSet;

pub type ResourceId = Handle<ResourceDesc>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Rgba8,
    Rgba16,
    R32F,
    Rg16,
    R8Array,
    D24S8,
    D32,
    D32S8,
    Rg11B10,
    Rgb10A2,
    Rg8,
    R8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    HardwareDepth,
    GBuffer,
    DepthCopy,
    HiZ,
    MotionVectors,
    ShadowMask,
    VolumetricFog,
    Lighting,
    PostProcess,
    History,
    Other,
}

impl ResourceKind {
    pub const fn allows_explicit_frame_clear(self) -> bool {
        matches!(
            self,
            Self::HardwareDepth | Self::VolumetricFog | Self::MotionVectors
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResourceDesc {
    pub name: String,
    pub format: Format,
    pub kind: ResourceKind,
    pub width_scale: f32,
    pub height_scale: f32,
    pub sample_count: u32,
}

impl ResourceDesc {
    pub fn new(name: impl Into<String>, format: Format, kind: ResourceKind) -> Self {
        Self {
            name: name.into(),
            format,
            kind,
            width_scale: 1.0,
            height_scale: 1.0,
            sample_count: 1,
        }
    }

    pub fn scaled(mut self, width_scale: f32, height_scale: f32) -> Self {
        self.width_scale = width_scale;
        self.height_scale = height_scale;
        self
    }

    pub fn multisampled(mut self, sample_count: u32) -> Self {
        self.sample_count = sample_count;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FeatureCategory {
    Visibility,
    Prepass,
    Geometry,
    Shadows,
    AmbientOcclusion,
    IndirectLighting,
    DirectLighting,
    Reflections,
    Volumetrics,
    PostProcess,
    AntiAliasing,
    Ui,
    Tooling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TemporalDependencyLevel {
    None,
    Optional,
    RequiredToHideDefects,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VendorDependencyLevel {
    None,
    OptionalAccelerator,
    RequiredForCorrectness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadOp {
    Load,
    Clear,
    DontCare,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StoreOp {
    Store,
    Discard,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentOp {
    pub resource: ResourceId,
    pub load: LoadOp,
    pub store: StoreOp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderPassDesc {
    pub name: String,
    pub owner: String,
    pub feature: FeatureCategory,
    pub resolution_scale: f32,
    pub target_formats: Vec<Format>,
    pub inputs: Vec<ResourceId>,
    pub outputs: Vec<ResourceId>,
    pub history_dependencies: Vec<ResourceId>,
    pub temporal_dependency: TemporalDependencyLevel,
    pub vendor_dependency: VendorDependencyLevel,
    pub timing_budget_ms: f32,
    pub attachments: Vec<AttachmentOp>,
    pub uses_stencil_light_projection: bool,
}

impl RenderPassDesc {
    pub fn new(
        name: impl Into<String>,
        owner: impl Into<String>,
        feature: FeatureCategory,
        timing_budget_ms: f32,
    ) -> Self {
        Self {
            name: name.into(),
            owner: owner.into(),
            feature,
            resolution_scale: 1.0,
            target_formats: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            history_dependencies: Vec::new(),
            temporal_dependency: TemporalDependencyLevel::None,
            vendor_dependency: VendorDependencyLevel::None,
            timing_budget_ms,
            attachments: Vec::new(),
            uses_stencil_light_projection: false,
        }
    }

    pub fn resolution_scale(mut self, scale: f32) -> Self {
        self.resolution_scale = scale;
        self
    }

    pub fn target_format(mut self, format: Format) -> Self {
        self.target_formats.push(format);
        self
    }

    pub fn read(mut self, resource: ResourceId) -> Self {
        self.inputs.push(resource);
        self
    }

    pub fn write(mut self, resource: ResourceId) -> Self {
        self.outputs.push(resource);
        self
    }

    pub fn history(mut self, resource: ResourceId) -> Self {
        self.history_dependencies.push(resource);
        self
    }

    pub fn attachment(mut self, resource: ResourceId, load: LoadOp, store: StoreOp) -> Self {
        self.attachments.push(AttachmentOp {
            resource,
            load,
            store,
        });
        self
    }

    pub fn temporal_dependency(mut self, level: TemporalDependencyLevel) -> Self {
        self.temporal_dependency = level;
        self
    }

    pub fn vendor_dependency(mut self, level: VendorDependencyLevel) -> Self {
        self.vendor_dependency = level;
        self
    }

    pub fn stencil_light_projection(mut self) -> Self {
        self.uses_stencil_light_projection = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GraphIssue {
    EmptyPassName,
    EmptyResourceName,
    DuplicatePassName {
        name: String,
    },
    InvalidTimingBudget {
        pass: String,
        budget_ms: f32,
    },
    InvalidResolutionScale {
        pass: String,
        scale: f32,
    },
    InvalidSampleCount {
        resource: String,
        sample_count: u32,
    },
    UnknownResource {
        pass: String,
        index: u32,
        generation: u32,
    },
    SuspiciousClear {
        pass: String,
        resource: String,
        kind: ResourceKind,
    },
    ShadowMaskClearWithStencil {
        pass: String,
        resource: String,
    },
    RequiredTemporalSmoothing {
        pass: String,
    },
    RequiredVendorFeature {
        pass: String,
    },
}

#[derive(Debug, Default)]
pub struct RenderGraph {
    resources: Arena<ResourceDesc>,
    passes: Vec<RenderPassDesc>,
}

impl RenderGraph {
    pub fn add_resource(&mut self, desc: ResourceDesc) -> ResourceId {
        self.resources.insert(desc)
    }

    pub fn add_pass(&mut self, pass: RenderPassDesc) {
        self.passes.push(pass);
    }

    pub fn resource(&self, id: ResourceId) -> Option<&ResourceDesc> {
        self.resources.get(id)
    }

    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    pub fn resource_count(&self) -> usize {
        self.resources.live_len()
    }

    pub fn resources(&self) -> impl Iterator<Item = &ResourceDesc> {
        self.resources.values()
    }

    pub fn passes(&self) -> &[RenderPassDesc] {
        &self.passes
    }

    pub fn validate(&self) -> Vec<GraphIssue> {
        let mut issues = Vec::new();
        let mut pass_names = HashSet::new();

        for resource in self.resources.values() {
            if !matches!(resource.sample_count, 1 | 2 | 4 | 8) {
                issues.push(GraphIssue::InvalidSampleCount {
                    resource: resource.name.clone(),
                    sample_count: resource.sample_count,
                });
            }
        }

        for pass in &self.passes {
            if pass.name.trim().is_empty() {
                issues.push(GraphIssue::EmptyPassName);
            } else if !pass_names.insert(pass.name.clone()) {
                issues.push(GraphIssue::DuplicatePassName {
                    name: pass.name.clone(),
                });
            }

            if !(pass.timing_budget_ms.is_finite() && pass.timing_budget_ms > 0.0) {
                issues.push(GraphIssue::InvalidTimingBudget {
                    pass: pass.name.clone(),
                    budget_ms: pass.timing_budget_ms,
                });
            }

            if !(pass.resolution_scale.is_finite()
                && pass.resolution_scale > 0.0
                && pass.resolution_scale <= 1.0)
            {
                issues.push(GraphIssue::InvalidResolutionScale {
                    pass: pass.name.clone(),
                    scale: pass.resolution_scale,
                });
            }

            if pass.temporal_dependency == TemporalDependencyLevel::RequiredToHideDefects {
                issues.push(GraphIssue::RequiredTemporalSmoothing {
                    pass: pass.name.clone(),
                });
            }

            if pass.vendor_dependency == VendorDependencyLevel::RequiredForCorrectness {
                issues.push(GraphIssue::RequiredVendorFeature {
                    pass: pass.name.clone(),
                });
            }

            for resource in pass
                .inputs
                .iter()
                .chain(pass.outputs.iter())
                .chain(pass.history_dependencies.iter())
            {
                if self.resource(*resource).is_none() {
                    let (index, generation) = resource.raw_parts();
                    issues.push(GraphIssue::UnknownResource {
                        pass: pass.name.clone(),
                        index,
                        generation,
                    });
                }
            }

            for attachment in &pass.attachments {
                let Some(resource) = self.resource(attachment.resource) else {
                    let (index, generation) = attachment.resource.raw_parts();
                    issues.push(GraphIssue::UnknownResource {
                        pass: pass.name.clone(),
                        index,
                        generation,
                    });
                    continue;
                };

                if resource.name.trim().is_empty() {
                    issues.push(GraphIssue::EmptyResourceName);
                }

                if attachment.load == LoadOp::Clear && !resource.kind.allows_explicit_frame_clear()
                {
                    issues.push(GraphIssue::SuspiciousClear {
                        pass: pass.name.clone(),
                        resource: resource.name.clone(),
                        kind: resource.kind,
                    });
                }

                if attachment.load == LoadOp::Clear
                    && resource.kind == ResourceKind::ShadowMask
                    && pass.uses_stencil_light_projection
                {
                    issues.push(GraphIssue::ShadowMaskClearWithStencil {
                        pass: pass.name.clone(),
                        resource: resource.name.clone(),
                    });
                }
            }
        }

        issues
    }
}

pub fn build_baseline_frame_graph() -> RenderGraph {
    let mut graph = RenderGraph::default();

    let hardware_depth = graph.add_resource(
        ResourceDesc::new(
            "2x-msaa-hardware-depth",
            Format::D32S8,
            ResourceKind::HardwareDepth,
        )
        .multisampled(2),
    );
    let motion_vectors = graph.add_resource(
        ResourceDesc::new(
            "2x-msaa-motion-vectors",
            Format::Rg16,
            ResourceKind::MotionVectors,
        )
        .multisampled(2),
    );
    let albedo = graph.add_resource(
        ResourceDesc::new(
            "2x-msaa-gbuffer-albedo",
            Format::Rgba8,
            ResourceKind::GBuffer,
        )
        .multisampled(2),
    );
    let normals = graph.add_resource(
        ResourceDesc::new(
            "2x-msaa-gbuffer-normals-material-flags",
            Format::Rgba16,
            ResourceKind::GBuffer,
        )
        .multisampled(2),
    );
    let pbr = graph.add_resource(
        ResourceDesc::new(
            "2x-msaa-gbuffer-pbr-material-id",
            Format::Rgba8,
            ResourceKind::GBuffer,
        )
        .multisampled(2),
    );
    let resolved_depth = graph.add_resource(ResourceDesc::new(
        "resolved-hardware-depth",
        Format::D32S8,
        ResourceKind::HardwareDepth,
    ));
    let resolved_motion = graph.add_resource(ResourceDesc::new(
        "resolved-motion-vectors",
        Format::Rg16,
        ResourceKind::MotionVectors,
    ));
    let resolved_albedo = graph.add_resource(ResourceDesc::new(
        "resolved-gbuffer-albedo",
        Format::Rgba8,
        ResourceKind::GBuffer,
    ));
    let resolved_normals = graph.add_resource(ResourceDesc::new(
        "resolved-gbuffer-normals-material-flags",
        Format::Rgba16,
        ResourceKind::GBuffer,
    ));
    let resolved_pbr = graph.add_resource(ResourceDesc::new(
        "resolved-gbuffer-pbr-material-id",
        Format::Rgba8,
        ResourceKind::GBuffer,
    ));
    let sample_coverage = graph.add_resource(ResourceDesc::new(
        "2x-msaa-resolved-sample-coverage",
        Format::R8,
        ResourceKind::Other,
    ));
    let depth_copy = graph.add_resource(ResourceDesc::new(
        "depth-copy-r32f",
        Format::R32F,
        ResourceKind::DepthCopy,
    ));
    let hiz = graph.add_resource(
        ResourceDesc::new(
            "hiz-min-max-discontinuity",
            Format::Rgba16,
            ResourceKind::HiZ,
        )
        .scaled(0.5, 0.5),
    );
    let directional_shadow = graph.add_resource(ResourceDesc::new(
        "cached-directional-shadow-map",
        Format::D32,
        ResourceKind::HardwareDepth,
    ));
    let local_shadows = graph.add_resource(ResourceDesc::new(
        "cached-local-shadow-map-array",
        Format::D32,
        ResourceKind::HardwareDepth,
    ));
    let environment_radiance = graph.add_resource(ResourceDesc::new(
        "scene-environment-radiance-rgba16f",
        Format::Rgba16,
        ResourceKind::Lighting,
    ));
    let lighting = graph.add_resource(ResourceDesc::new(
        "hdr-lighting-rgba16f",
        Format::Rgba16,
        ResourceKind::Lighting,
    ));
    let tone_mapped = graph.add_resource(ResourceDesc::new(
        "tone-mapped-rgb10a2",
        Format::Rgb10A2,
        ResourceKind::PostProcess,
    ));
    let smaa_edges = graph.add_resource(ResourceDesc::new(
        "smaa-edge-mask-rg8",
        Format::Rg8,
        ResourceKind::PostProcess,
    ));
    let ssr = graph.add_resource(
        ResourceDesc::new(
            "half-resolution-ssr",
            Format::Rgba16,
            ResourceKind::Lighting,
        )
        .scaled(0.5, 0.5),
    );
    let software_rt = graph.add_resource(
        ResourceDesc::new(
            "software-rt-reflections-70-percent",
            Format::Rgba16,
            ResourceKind::Lighting,
        )
        .scaled(0.7, 0.7),
    );
    let rt_history = graph.add_resource(
        ResourceDesc::new(
            "two-frame-rt-history",
            Format::Rgba16,
            ResourceKind::History,
        )
        .scaled(0.7, 0.7),
    );
    let volumetric_fog = graph.add_resource(
        ResourceDesc::new(
            "blue-noise-volumetric-fog",
            Format::Rgba16,
            ResourceKind::VolumetricFog,
        )
        .scaled(0.5, 0.5),
    );
    let bloom = graph.add_resource(
        ResourceDesc::new(
            "diffuse-gaussian-bloom-rg11b10",
            Format::Rg11B10,
            ResourceKind::PostProcess,
        )
        .scaled(0.25, 0.25),
    );
    let exposure_white_balance = graph.add_resource(
        ResourceDesc::new(
            "auto-exposure-white-balance-1x1",
            Format::Rgba16,
            ResourceKind::PostProcess,
        )
        .scaled(1.0 / 1920.0, 1.0 / 1080.0),
    );
    let smaa_blend = graph.add_resource(ResourceDesc::new(
        "smaa-blend-mask-rgba16",
        Format::Rgba16,
        ResourceKind::PostProcess,
    ));
    let velocity_a = graph.add_resource(ResourceDesc::new(
        "single-direction-velocity-compensation-a",
        Format::Rgba16,
        ResourceKind::PostProcess,
    ));
    let velocity_b = graph.add_resource(ResourceDesc::new(
        "separable-velocity-compensation-b",
        Format::Rgba16,
        ResourceKind::PostProcess,
    ));
    let aa_history = graph.add_resource(ResourceDesc::new(
        "two-frame-spatial-smaa-history",
        Format::Rgb10A2,
        ResourceKind::History,
    ));
    let aa_presented = graph.add_resource(ResourceDesc::new(
        "anti-aliased-scene-presentation",
        Format::Rgb10A2,
        ResourceKind::PostProcess,
    ));
    let ui_overlay = graph.add_resource(ResourceDesc::new(
        "separate-linear-ui-overlay",
        Format::Rgba8,
        ResourceKind::PostProcess,
    ));
    let presented = graph.add_resource(ResourceDesc::new(
        "gamma-correct-scene-ui-presentation",
        Format::Rgba8,
        ResourceKind::PostProcess,
    ));

    graph.add_pass(
        RenderPassDesc::new(
            "cached-alpha-aware-directional-and-local-shadow-maps",
            "shadows",
            FeatureCategory::Shadows,
            1.4,
        )
        .write(directional_shadow)
        .write(local_shadows)
        .attachment(directional_shadow, LoadOp::Clear, StoreOp::Store)
        .attachment(local_shadows, LoadOp::Clear, StoreOp::Store),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "frame-start-clears",
            "renderer",
            FeatureCategory::Visibility,
            0.1,
        )
        .write(hardware_depth)
        .write(motion_vectors)
        .attachment(hardware_depth, LoadOp::Clear, StoreOp::Store)
        .attachment(motion_vectors, LoadOp::Clear, StoreOp::Store),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "partial-depth-prepass",
            "visibility",
            FeatureCategory::Prepass,
            0.5,
        )
        .write(hardware_depth)
        .attachment(hardware_depth, LoadOp::Load, StoreOp::Store),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "base-pass-deferred-materials",
            "geometry",
            FeatureCategory::Geometry,
            3.0,
        )
        .read(hardware_depth)
        .write(albedo)
        .write(normals)
        .write(pbr)
        .write(motion_vectors)
        .target_format(Format::Rgba8)
        .target_format(Format::Rgba16)
        .target_format(Format::Rgba8)
        .target_format(Format::Rg16)
        .attachment(hardware_depth, LoadOp::Load, StoreOp::Store),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "2x-msaa-gbuffer-custom-coverage-resolve",
            "geometry",
            FeatureCategory::AntiAliasing,
            0.45,
        )
        .read(hardware_depth)
        .read(albedo)
        .read(normals)
        .read(pbr)
        .read(motion_vectors)
        .write(resolved_depth)
        .write(resolved_albedo)
        .write(resolved_normals)
        .write(resolved_pbr)
        .write(resolved_motion)
        .write(sample_coverage)
        .target_format(Format::Rgba8)
        .target_format(Format::Rgba16)
        .target_format(Format::Rgba8)
        .target_format(Format::Rg16)
        .target_format(Format::R8)
        .attachment(resolved_depth, LoadOp::Clear, StoreOp::Store),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "depth-copy-fullscreen-triangle",
            "visibility",
            FeatureCategory::Visibility,
            0.2,
        )
        .read(resolved_depth)
        .write(depth_copy)
        .target_format(Format::R32F),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "hiz-generation-compute",
            "visibility",
            FeatureCategory::Visibility,
            0.3,
        )
        .read(depth_copy)
        .write(hiz)
        .target_format(Format::Rgba16),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "optimized-half-resolution-ssr",
            "reflections",
            FeatureCategory::Reflections,
            0.7,
        )
        .resolution_scale(0.5)
        .read(depth_copy)
        .read(resolved_albedo)
        .read(resolved_normals)
        .read(resolved_pbr)
        .write(ssr)
        .target_format(Format::Rgba16),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "compute-software-rt-reflection-supplement",
            "reflections",
            FeatureCategory::Reflections,
            1.3,
        )
        .resolution_scale(0.7)
        .read(depth_copy)
        .read(hiz)
        .read(resolved_albedo)
        .read(resolved_normals)
        .read(resolved_pbr)
        .read(directional_shadow)
        .read(local_shadows)
        .read(ssr)
        .read(resolved_motion)
        .read(environment_radiance)
        .history(rt_history)
        .write(software_rt)
        .target_format(Format::Rgba16)
        .temporal_dependency(TemporalDependencyLevel::Optional),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "blue-noise-volumetric-fog",
            "post",
            FeatureCategory::Volumetrics,
            1.0,
        )
        .read(depth_copy)
        .read(environment_radiance)
        .write(volumetric_fog)
        .target_format(Format::Rgba16)
        .attachment(volumetric_fog, LoadOp::Clear, StoreOp::Store),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "local-light-z-fail-stencil-volumes",
            "lighting",
            FeatureCategory::DirectLighting,
            0.25,
        )
        .read(resolved_depth)
        .attachment(resolved_depth, LoadOp::Load, StoreOp::Store)
        .stencil_light_projection(),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "material-profile-directional-lighting-fused-ao-contact-shadows",
            "lighting",
            FeatureCategory::DirectLighting,
            1.5,
        )
        .read(resolved_albedo)
        .read(resolved_normals)
        .read(resolved_pbr)
        .read(depth_copy)
        .read(hiz)
        .read(directional_shadow)
        .read(local_shadows)
        .read(ssr)
        .read(software_rt)
        .read(environment_radiance)
        .write(lighting)
        .target_format(Format::Rgba16),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "stencil-restricted-local-lighting",
            "lighting",
            FeatureCategory::DirectLighting,
            0.75,
        )
        .read(resolved_depth)
        .read(resolved_albedo)
        .read(resolved_normals)
        .read(resolved_pbr)
        .read(directional_shadow)
        .read(local_shadows)
        .write(lighting)
        .target_format(Format::Rgba16)
        .attachment(resolved_depth, LoadOp::Load, StoreOp::Store)
        .stencil_light_projection(),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "diffuse-luminance-auto-exposure-white-balance",
            "post",
            FeatureCategory::PostProcess,
            0.2,
        )
        .read(lighting)
        .write(exposure_white_balance)
        .target_format(Format::Rgba16),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "world-space-depth-tested-rain",
            "weather",
            FeatureCategory::Volumetrics,
            0.45,
        )
        .read(resolved_depth)
        .read(directional_shadow)
        .read(local_shadows)
        .read(environment_radiance)
        .read(lighting)
        .write(lighting)
        .target_format(Format::Rgba16),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "single-direction-depth-aware-velocity-compensation",
            "post",
            FeatureCategory::PostProcess,
            0.7,
        )
        .read(lighting)
        .read(resolved_motion)
        .read(resolved_depth)
        .write(velocity_a)
        .target_format(Format::Rgba16),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "separable-velocity-compensation-smoothing",
            "post",
            FeatureCategory::PostProcess,
            0.35,
        )
        .read(velocity_a)
        .read(resolved_motion)
        .read(resolved_depth)
        .write(velocity_b)
        .target_format(Format::Rgba16),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "diffuse-gaussian-bloom-chain",
            "post",
            FeatureCategory::PostProcess,
            0.6,
        )
        .read(velocity_b)
        .write(bloom)
        .target_format(Format::Rg11B10),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "gt-tonemap-color-resolve",
            "post",
            FeatureCategory::PostProcess,
            1.0,
        )
        .read(velocity_b)
        .read(volumetric_fog)
        .read(bloom)
        .read(exposure_white_balance)
        .write(tone_mapped)
        .target_format(Format::Rgb10A2),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "smaa-compute-edge-detect",
            "anti-aliasing",
            FeatureCategory::AntiAliasing,
            0.5,
        )
        .read(tone_mapped)
        .read(resolved_depth)
        .read(sample_coverage)
        .write(smaa_edges)
        .target_format(Format::Rg8),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "smaa-blend-weights",
            "anti-aliasing",
            FeatureCategory::AntiAliasing,
            0.5,
        )
        .read(smaa_edges)
        .read(tone_mapped)
        .read(resolved_depth)
        .write(smaa_blend)
        .target_format(Format::Rgba16),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "smaa-t2x-motion-vector-resolve",
            "anti-aliasing",
            FeatureCategory::AntiAliasing,
            0.5,
        )
        .read(tone_mapped)
        .read(smaa_blend)
        .read(resolved_depth)
        .read(resolved_normals)
        .read(resolved_motion)
        .history(aa_history)
        .write(aa_history)
        .write(aa_presented)
        .target_format(Format::Rgb10A2)
        .temporal_dependency(TemporalDependencyLevel::Optional),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "separate-linear-ui-overlay",
            "ui",
            FeatureCategory::Ui,
            0.25,
        )
        .write(ui_overlay)
        .target_format(Format::Rgba8),
    );

    graph.add_pass(
        RenderPassDesc::new(
            "gamma-correct-scene-ui-presentation",
            "ui",
            FeatureCategory::Ui,
            0.25,
        )
        .read(aa_presented)
        .read(ui_overlay)
        .write(presented)
        .target_format(Format::Rgba8),
    );

    graph
}
