use dhool_pro::asset::{
    validate_asset, AssetIssue, MaterialDesc, MeshDesc, ShadowCasterClass, TextureDesc,
    TextureFormat, TextureRole,
};
use dhool_pro::graph::{
    build_baseline_frame_graph, FeatureCategory, Format, GraphIssue, LoadOp, RenderGraph,
    RenderPassDesc, ResourceDesc, ResourceKind, StoreOp, VendorDependencyLevel,
};
use dhool_pro::memory::Arena;
use dhool_pro::perf::{blue_noise_hash, edge_stencil_4x4, reduce_hiz_rgba16};
use dhool_pro::quality::QualityTier;

fn quantize(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0).round() as u16
}

#[test]
fn baseline_frame_graph_obeys_spec_constraints() {
    let graph = build_baseline_frame_graph();
    assert_eq!(graph.validate(), Vec::new());
    assert!(graph.pass_count() >= 8);
    assert!(graph.resource_count() >= 10);
    let pass_names: Vec<_> = graph
        .passes()
        .iter()
        .map(|pass| pass.name.as_str())
        .collect();
    for required in [
        "2x-msaa-gbuffer-custom-coverage-resolve",
        "local-light-z-fail-stencil-volumes",
        "stencil-restricted-local-lighting",
        "world-space-depth-tested-rain",
        "single-direction-depth-aware-velocity-compensation",
        "diffuse-gaussian-bloom-chain",
        "smaa-t2x-motion-vector-resolve",
        "separate-linear-ui-overlay",
        "gamma-correct-scene-ui-presentation",
    ] {
        assert!(pass_names.contains(&required), "missing {required}");
    }
    assert!(graph
        .resources()
        .any(|resource| resource.name == "2x-msaa-hardware-depth" && resource.sample_count == 2));
    assert!(graph
        .resources()
        .any(|resource| resource.name == "scene-environment-radiance-rgba16f"));
    assert!(!pass_names.contains(&"half-resolution-camera-composite-effects"));
    assert!(!graph
        .resources()
        .any(|resource| resource.name == "half-resolution-camera-composite-effects"));
}

#[test]
fn live_shader_contract_contains_real_coverage_stencil_and_history_paths() {
    let resolve = include_str!("../src/shaders/gbuffer_resolve.wgsl");
    assert!(resolve.contains("texture_multisampled_2d"));
    assert!(resolve.contains("sample_coverage"));

    let lighting = include_str!("../src/shaders/lighting.wgsl");
    assert!(lighting.contains("fs_local_lights"));
    assert!(lighting.contains("material_profiles"));
    assert!(lighting.contains("fn environment_irradiance"));
    assert!(lighting.contains("var skybox: texture_2d<f32>"));

    let reflections = include_str!("../src/shaders/reflections.wgsl");
    assert!(reflections.contains("fn environment_radiance"));
    assert!(reflections.contains("textureSampleLevel(skybox"));

    let fog = include_str!("../src/shaders/volumetric_fog.wgsl");
    assert!(fog.contains("fn environment_radiance"));
    assert!(fog.contains("textureSampleLevel(skybox"));

    let rain = include_str!("../src/shaders/rain3d.wgsl");
    assert!(rain.contains("fn environment_radiance"));
    assert!(rain.contains("directional_fill"));

    let aa = include_str!("../src/shaders/aa.wgsl");
    assert!(aa.contains("spatial_resolve"));
    assert!(!aa.contains("fps_overlay"));

    let presentation = include_str!("../src/shaders/present.wgsl");
    assert!(presentation.contains("display_gamma"));
    assert!(presentation.contains("ui_overlay"));
}

#[test]
fn every_wgsl_shader_parses_and_validates() {
    let shader_directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
    let mut shader_paths = std::fs::read_dir(&shader_directory)
        .expect("shader directory")
        .map(|entry| entry.expect("shader directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "wgsl")
        })
        .collect::<Vec<_>>();
    shader_paths.sort();
    assert!(!shader_paths.is_empty());

    for path in shader_paths {
        let source = std::fs::read_to_string(&path).expect("WGSL source");
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{} failed to parse: {error}", path.display()));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{} failed validation: {error}", path.display()));
    }
}

#[test]
fn graph_rejects_shadow_mask_clear_under_stencil_projection() {
    let mut graph = RenderGraph::default();
    let shadow_mask = graph.add_resource(ResourceDesc::new(
        "shadow",
        Format::R8Array,
        ResourceKind::ShadowMask,
    ));
    graph.add_pass(
        RenderPassDesc::new("bad-shadow", "test", FeatureCategory::Shadows, 0.1)
            .write(shadow_mask)
            .attachment(shadow_mask, LoadOp::Clear, StoreOp::Store)
            .stencil_light_projection(),
    );

    let issues = graph.validate();
    assert!(issues
        .iter()
        .any(|issue| matches!(issue, GraphIssue::ShadowMaskClearWithStencil { .. })));
    assert!(issues
        .iter()
        .any(|issue| matches!(issue, GraphIssue::SuspiciousClear { .. })));
}

#[test]
fn graph_rejects_vendor_required_correctness() {
    let mut graph = RenderGraph::default();
    graph.add_pass(
        RenderPassDesc::new(
            "dxr-only-reflections",
            "test",
            FeatureCategory::Reflections,
            0.5,
        )
        .vendor_dependency(VendorDependencyLevel::RequiredForCorrectness),
    );

    assert!(graph
        .validate()
        .iter()
        .any(|issue| matches!(issue, GraphIssue::RequiredVendorFeature { .. })));
}

#[test]
fn asset_validation_catches_spec_antipatterns() {
    let materials = vec![MaterialDesc {
        name: "bad foliage".to_string(),
        textures: vec![
            TextureDesc {
                name: "normal_bc5".to_string(),
                format: TextureFormat::Bc5,
                role: TextureRole::NormalsAndBentNormals,
            },
            TextureDesc {
                name: "packed_alpha".to_string(),
                format: TextureFormat::Bc3,
                role: TextureRole::BaseColorAndAo,
            },
        ],
        texture_sample_count: 6,
        runtime_texture_blending: true,
        requires_specular_occlusion: true,
        has_bent_normals: false,
        alpha_tested: true,
    }];

    let meshes = vec![MeshDesc {
        name: "floor leaves".to_string(),
        material_index: 0,
        is_floor_or_ground: true,
        shadow_class: ShadowCasterClass::Main,
        participates_in_prepass: true,
        writes_motion_vectors_in_prepass: true,
        long_thin_triangles: true,
        dense_alpha_cards: true,
    }];

    let issues = validate_asset(&materials, &meshes, QualityTier::High);
    assert!(issues.iter().any(|issue| matches!(
        issue,
        AssetIssue::TooManyMaterialTextures {
            count: 6,
            limit: 4,
            ..
        }
    )));
    assert!(issues
        .iter()
        .any(|issue| matches!(issue, AssetIssue::Bc5NormalMap { .. })));
    assert!(issues
        .iter()
        .any(|issue| matches!(issue, AssetIssue::MissingSeparateBc4Opacity { .. })));
    assert!(issues
        .iter()
        .any(|issue| matches!(issue, AssetIssue::FloorCastsShadow { .. })));
    assert!(issues
        .iter()
        .any(|issue| matches!(issue, AssetIssue::AlphaTestedInPrepass { .. })));
    assert!(issues
        .iter()
        .any(|issue| matches!(issue, AssetIssue::MotionVectorsInPrepass { .. })));
}

#[test]
fn arena_invalidates_stale_handles() {
    let mut arena = Arena::default();
    let first = arena.insert(String::from("depth"));
    assert_eq!(arena.get(first).map(String::as_str), Some("depth"));
    assert_eq!(arena.remove(first).as_deref(), Some("depth"));
    assert!(arena.get(first).is_none());

    let second = arena.insert(String::from("motion"));
    assert_ne!(first.raw_parts(), second.raw_parts());
    assert_eq!(arena.get(second).map(String::as_str), Some("motion"));
}

#[test]
fn cpp_hiz_kernel_reduces_depth_to_min_max_tiles() {
    let depth = [0.10, 0.30, 0.40, 0.20];
    let reduced = reduce_hiz_rgba16(&depth, 2, 2).expect("hiz reduction");
    assert_eq!(reduced.len(), 1);
    assert_eq!(
        reduced[0],
        [quantize(0.10), quantize(0.40), quantize(0.30), 65535]
    );
}

#[test]
fn cpp_edge_stencil_flags_4x4_discontinuities() {
    let mut depth = vec![0.5; 16];
    depth[15] = 0.9;
    let mask = edge_stencil_4x4(&depth, 4, 4, 0.05).expect("edge stencil");
    assert_eq!(mask, vec![1]);

    let flat = edge_stencil_4x4(&[0.5; 16], 4, 4, 0.05).expect("flat stencil");
    assert_eq!(flat, vec![0]);
}

#[test]
fn cpp_blue_noise_hash_changes_with_frame() {
    let a = blue_noise_hash(12, 8, 0, 1337);
    let b = blue_noise_hash(12, 8, 1, 1337);
    assert_ne!(a, b);
    assert_eq!(a, blue_noise_hash(12, 8, 0, 1337));
}
