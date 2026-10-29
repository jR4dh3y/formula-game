use std::env;

fn main() {
    println!("cargo:rerun-if-changed=cpp/perf_kernels.cpp");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("cpp/perf_kernels.cpp")
        .warnings(true);

    if target_os == "windows" {
        build.define("NOMINMAX", None);
        build.define("WIN32_LEAN_AND_MEAN", None);
    }

    if target_env == "msvc" {
        build.flag_if_supported("/permissive-");
        build.flag_if_supported("/EHsc");
    } else {
        build.flag_if_supported("-fno-exceptions");
        build.flag_if_supported("-fno-rtti");
        build.flag_if_supported("-fvisibility=hidden");
    }

    build.compile("gospol_perf");
}
