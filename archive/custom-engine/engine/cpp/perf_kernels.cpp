#if defined(_MSC_VER) && defined(_WIN64)
using usize = unsigned __int64;
#elif defined(_MSC_VER)
using usize = unsigned int;
#else
using usize = __SIZE_TYPE__;
#endif
using u8 = unsigned char;
using u16 = unsigned short;
using u32 = unsigned int;

namespace {
float clamp01(float value) {
    if (value < 0.0f) {
        return 0.0f;
    }
    if (value > 1.0f) {
        return 1.0f;
    }
    return value;
}

float sanitize_depth(float value) {
    constexpr float max_float = 3.4028234663852886e+38f;
    if (!(value == value) || value > max_float || value < -max_float) {
        return 1.0f;
    }
    return clamp01(value);
}

u16 quantize_unorm16(float value) {
    value = clamp01(value);
    return static_cast<u16>(value * 65535.0f + 0.5f);
}

u32 hash_u32(u32 x) {
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x;
}
} // namespace

extern "C" usize gospol_hiz_reduce_rgba16(
    const float* depth,
    u32 width,
    u32 height,
    u16* out_rgba16,
    usize out_len) {
    if (depth == nullptr || out_rgba16 == nullptr || width == 0 || height == 0) {
        return 0;
    }

    const u32 out_width = (width + 1u) / 2u;
    const u32 out_height = (height + 1u) / 2u;
    const usize cells = static_cast<usize>(out_width) * out_height;
    if (out_len < cells * 4u) {
        return 0;
    }

    for (u32 oy = 0; oy < out_height; ++oy) {
        for (u32 ox = 0; ox < out_width; ++ox) {
            float min_depth = 1.0f;
            float max_depth = 0.0f;
            u32 covered = 0;

            for (u32 dy = 0; dy < 2u; ++dy) {
                const u32 y = oy * 2u + dy;
                if (y >= height) {
                    continue;
                }
                for (u32 dx = 0; dx < 2u; ++dx) {
                    const u32 x = ox * 2u + dx;
                    if (x >= width) {
                        continue;
                    }
                    const float z = sanitize_depth(depth[static_cast<usize>(y) * width + x]);
                    min_depth = z < min_depth ? z : min_depth;
                    max_depth = z > max_depth ? z : max_depth;
                    ++covered;
                }
            }

            const usize i = (static_cast<usize>(oy) * out_width + ox) * 4u;
            out_rgba16[i + 0u] = quantize_unorm16(min_depth);
            out_rgba16[i + 1u] = quantize_unorm16(max_depth);
            out_rgba16[i + 2u] = quantize_unorm16(max_depth - min_depth);
            out_rgba16[i + 3u] = static_cast<u16>((covered * 65535u) / 4u);
        }
    }

    return cells;
}

extern "C" usize gospol_edge_stencil_4x4(
    const float* depth,
    u32 width,
    u32 height,
    float threshold,
    u8* out_mask,
    usize out_len) {
    if (depth == nullptr || out_mask == nullptr || width == 0 || height == 0) {
        return 0;
    }

    const u32 out_width = (width + 3u) / 4u;
    const u32 out_height = (height + 3u) / 4u;
    const usize tiles = static_cast<usize>(out_width) * out_height;
    if (out_len < tiles) {
        return 0;
    }

    threshold = threshold > 0.0f ? threshold : 0.0f;

    for (u32 ty = 0; ty < out_height; ++ty) {
        for (u32 tx = 0; tx < out_width; ++tx) {
            float min_depth = 1.0f;
            float max_depth = 0.0f;

            for (u32 dy = 0; dy < 4u; ++dy) {
                const u32 y = ty * 4u + dy;
                if (y >= height) {
                    continue;
                }
                for (u32 dx = 0; dx < 4u; ++dx) {
                    const u32 x = tx * 4u + dx;
                    if (x >= width) {
                        continue;
                    }
                    const float z = sanitize_depth(depth[static_cast<usize>(y) * width + x]);
                    min_depth = z < min_depth ? z : min_depth;
                    max_depth = z > max_depth ? z : max_depth;
                }
            }

            out_mask[static_cast<usize>(ty) * out_width + tx] =
                (max_depth - min_depth) > threshold ? 1u : 0u;
        }
    }

    return tiles;
}

extern "C" u32 gospol_blue_noise_hash(
    u32 x,
    u32 y,
    u32 frame,
    u32 seed) {
    u32 value = seed;
    value ^= x * 0x9e3779b9u;
    value ^= y * 0x85ebca6bu;
    value ^= frame * 0xc2b2ae35u;
    return hash_u32(value);
}

extern "C" usize gospol_gt_tonemap_rgba8(
    const float* hdr_rgba,
    u32 pixel_count,
    float exposure,
    u8* out_rgba8,
    usize out_len) {
    if (hdr_rgba == nullptr || out_rgba8 == nullptr) {
        return 0;
    }

    const usize required = static_cast<usize>(pixel_count) * 4u;
    if (out_len < required) {
        return 0;
    }

    exposure = exposure > 0.0f ? exposure : 1.0f;
    for (u32 i = 0; i < pixel_count; ++i) {
        const usize p = static_cast<usize>(i) * 4u;
        for (u32 c = 0; c < 3u; ++c) {
            const float x = hdr_rgba[p + c] > 0.0f ? hdr_rgba[p + c] * exposure : 0.0f;
            const float shoulder = x * (2.51f * x + 0.03f);
            const float toe = x * (2.43f * x + 0.59f) + 0.14f;
            const float mapped = clamp01(shoulder / toe);
            out_rgba8[p + c] = static_cast<u8>(mapped * 255.0f + 0.5f);
        }
        out_rgba8[p + 3u] = static_cast<u8>(clamp01(hdr_rgba[p + 3u]) * 255.0f + 0.5f);
    }

    return pixel_count;
}

extern "C" usize gospol_kawase_blur_rgba32f(
    const float* src_rgba,
    u32 width,
    u32 height,
    u32 radius,
    float* dst_rgba,
    usize out_len) {
    if (src_rgba == nullptr || dst_rgba == nullptr || width == 0 || height == 0) {
        return 0;
    }

    const usize pixels = static_cast<usize>(width) * height;
    if (out_len < pixels * 4u) {
        return 0;
    }

    const u32 r = radius > 0u ? radius : 1u;
    for (u32 y = 0; y < height; ++y) {
        for (u32 x = 0; x < width; ++x) {
            const u32 xs[5] = {
                x,
                x > r ? x - r : 0u,
                x + r < width ? x + r : width - 1u,
                x > r ? x - r : 0u,
                x + r < width ? x + r : width - 1u,
            };
            const u32 ys[5] = {
                y,
                y > r ? y - r : 0u,
                y > r ? y - r : 0u,
                y + r < height ? y + r : height - 1u,
                y + r < height ? y + r : height - 1u,
            };
            const usize out = (static_cast<usize>(y) * width + x) * 4u;
            for (u32 c = 0; c < 4u; ++c) {
                float sum = 0.0f;
                for (u32 s = 0; s < 5u; ++s) {
                    const usize in = (static_cast<usize>(ys[s]) * width + xs[s]) * 4u;
                    sum += src_rgba[in + c];
                }
                dst_rgba[out + c] = sum * 0.2f;
            }
        }
    }

    return pixels;
}
