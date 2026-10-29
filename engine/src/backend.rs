#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendApi {
    Vulkan13,
    Vulkan14,
    Direct3D12,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub api: BackendApi,
    pub explicit_resource_states: bool,
    pub descriptor_indexing: bool,
    pub pipeline_cache: bool,
    pub msaa_sample_positions: bool,
    pub conservative_rasterization: bool,
    pub timestamp_queries: bool,
    pub pipeline_statistics: bool,
    pub crash_diagnostics: bool,
    pub hardware_ray_tracing_optional: bool,
}

impl BackendCapabilities {
    pub const fn baseline(api: BackendApi) -> Self {
        Self {
            api,
            explicit_resource_states: true,
            descriptor_indexing: true,
            pipeline_cache: true,
            msaa_sample_positions: true,
            conservative_rasterization: true,
            timestamp_queries: true,
            pipeline_statistics: true,
            crash_diagnostics: true,
            hardware_ray_tracing_optional: true,
        }
    }
}

pub fn supported_backends_for_target(target_os: &str) -> Vec<BackendApi> {
    match target_os {
        "windows" => vec![
            BackendApi::Direct3D12,
            BackendApi::Vulkan14,
            BackendApi::Vulkan13,
        ],
        "linux" => vec![BackendApi::Vulkan14, BackendApi::Vulkan13],
        _ => vec![BackendApi::Vulkan13],
    }
}
