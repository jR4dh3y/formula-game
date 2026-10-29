struct Lights {
    directional_direction_count: vec4<f32>,
    directional_color_intensity: vec4<f32>,
    ambient: vec4<f32>,
    sky: vec4<f32>,
    weather: vec4<f32>,
    environment: vec4<f32>,
    environment_sh: array<vec4<f32>, 9>,
    local_position_radius: array<vec4<f32>, 6>,
    local_color_intensity: array<vec4<f32>, 6>,
    local_direction_outer: array<vec4<f32>, 6>,
    local_inner_shadow: array<vec4<f32>, 6>,
};

struct ExposureSettings {
    limits_compensation_balance: vec4<f32>,
    adaptation_speeds: vec4<f32>,
};

struct AntiAlias {
    previous_view_projection: mat4x4<f32>,
    viewport: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var lighting_hdr: texture_2d<f32>;
@group(0) @binding(1) var exposure_white_balance: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> lights: Lights;
@group(0) @binding(3) var previous_calibration: texture_2d<f32>;
@group(0) @binding(4) var<uniform> settings: ExposureSettings;
@group(0) @binding(5) var<uniform> frame_timing: AntiAlias;

var<workgroup> log_luminance: array<f32, 64>;

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(
    @builtin(local_invocation_index) local_index: u32,
    @builtin(local_invocation_id) local_id: vec3<u32>
) {
    let size = textureDimensions(lighting_hdr);
    var sum = 0.0;
    var count = 0u;
    // Stratified 64x64 metering grid. Reading every screen pixel from one
    // workgroup serializes millions of texture loads and scales with resolution.
    // Every lane takes the same number of samples, including on tiny viewports.
    for (var y = local_id.y; y < 64u; y += 8u) {
        for (var x = local_id.x; x < 64u; x += 8u) {
            let uv = (vec2<f32>(f32(x), f32(y)) + 0.5) / 64.0;
            let pixel = min(vec2<u32>(uv * vec2<f32>(size)), size - vec2<u32>(1u));
            let diffuse_illuminance = textureLoad(lighting_hdr, vec2<i32>(pixel), 0).a;
            sum += log(max(diffuse_illuminance, 0.0001));
            count += 1u;
        }
    }
    log_luminance[local_index] = sum / f32(max(count, 1u));
    workgroupBarrier();

    var stride = 32u;
    loop {
        if local_index < stride {
            log_luminance[local_index] += log_luminance[local_index + stride];
        }
        workgroupBarrier();
        if stride == 1u { break; }
        stride /= 2u;
    }

    if local_index == 0u {
        let geometric_mean = exp(log_luminance[0] / 64.0);
        let exposure_limits = settings.limits_compensation_balance.xy;
        let compensation = exp2(settings.limits_compensation_balance.z);
        let target_exposure = clamp(
            0.18 * compensation / max(geometric_mean, 0.0001),
            exposure_limits.x,
            exposure_limits.y
        );

        let illuminant = lights.ambient.rgb + lights.sky.rgb * 0.35;
        let neutral = max(luminance(illuminant), 0.0001);
        let target_gains = clamp(
            vec3<f32>(neutral) / max(illuminant, vec3<f32>(0.0001)),
            vec3<f32>(0.92),
            vec3<f32>(1.08)
        );
        let balance_strength = min(settings.limits_compensation_balance.w, 0.65);
        let balanced_target = mix(vec3<f32>(1.0), target_gains, balance_strength);
        let previous = textureLoad(previous_calibration, vec2<i32>(0), 0);
        let has_history = previous.a > 0.0001;
        let fps = max(frame_timing.params.w, 1.0);
        let speed = select(
            settings.adaptation_speeds.y,
            settings.adaptation_speeds.x,
            target_exposure > previous.a
        );
        let adaptation = 1.0 - exp(-speed / fps);
        let exposure = select(target_exposure, mix(previous.a, target_exposure, adaptation), has_history);
        let gains = select(balanced_target, mix(previous.rgb, balanced_target, adaptation), has_history);
        textureStore(exposure_white_balance, vec2<i32>(0), vec4<f32>(gains, exposure));
    }
}
