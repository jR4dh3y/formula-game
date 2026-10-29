#![deny(unsafe_op_in_unsafe_fn)]
//! Reusable game runtime and vendor-agnostic renderer.
//!
//! Start with [`SceneBuilder`] and [`GameEngine::builder`].

pub mod asset;
pub mod asset_pipeline;
pub mod backend;
pub mod cloth;
pub mod game;
pub mod gltf_import;
pub mod gpu;
pub mod graph;
pub mod math;
pub mod memory;
pub mod perf;
pub mod profiler;
pub mod quality;
pub mod renderer;
pub mod runtime;
pub mod scene;
pub mod scene_builder;
pub mod world;

pub use asset::{
    validate_asset, validate_material, validate_mesh, AssetIssue, MaterialDesc, MeshDesc,
    ShadowCasterClass,
};
pub use asset_pipeline::{AssetId, AssetPipeline, ImportReport, ImportSeverity, ImportedAsset};
pub use backend::{BackendApi, BackendCapabilities};
pub use cloth::{
    ClothColliderSnapshot, ClothDefinition, ClothError, ClothPin, ClothSettings, ClothState,
};
pub use game::{
    EngineBuildError, EngineError, EngineFrame, EnginePlugin, EngineUpdate, FlyCameraPlugin,
    FrameInput, GameEngine, GameEngineBuilder, InputState, ProjectSettings, TargetPlatform,
};
pub use gltf_import::{
    import_gltf, import_gltf_scene, GltfImportError, GltfImportOptions, GltfNode, GltfScene,
};
pub use gpu::{run_live_game, run_live_renderer, LiveError, LiveOptions};
pub use graph::{build_baseline_frame_graph, FeatureCategory, GraphIssue, RenderGraph};
pub use profiler::{required_debug_views, EngineProfiler, FrameBudget, FrameReport, GateFailure};
pub use quality::QualityTier;
pub use renderer::{CpuRenderer, DebugView, RenderConfig, RenderOutput, RenderPath};
pub use runtime::{
    Events, GameSystem, Resources, Schedule, SystemContext, SystemFailure, SystemResult, Time,
};
pub use scene::{
    Camera, EnvironmentEncoding, EnvironmentMap, EnvironmentProfile, ExposureProfile, Light,
    LightKind, Material, MaterialClass, MaterialFlags, Mesh, RenderScene, TextureImage,
    TexturePixels, Vertex,
};
pub use scene_builder::{MaterialId, SceneBuildError, SceneBuilder};
pub use world::{
    CameraRig, Collider, EntityId, LightComponent, MeshRenderer, Name, PhysicsSettings, RigidBody,
    ScriptState, Transform, World, WorldIssue,
};
