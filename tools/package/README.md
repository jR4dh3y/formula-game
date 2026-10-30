F1 Racecraft — Linux x86-64 portable build

Make the AppImage executable and open it:

    chmod +x F1-Racecraft-0.1.0-linux-x86_64.AppImage
    ./F1-Racecraft-0.1.0-linux-x86_64.AppImage

Only that AppImage needs to be sent to another player. It contains the game,
RB6/GEN22, Spa, engine audio, fonts, runtime libraries
and a matching libc/loader. No Rust, separate assets, installer, root access,
network download or FUSE installation is needed. When mounting is unavailable,
the embedded runtime extracts to temporary storage and cleans up after exit.
Keep approximately 600 MB of temporary space free.

Requires an x86-64 Linux desktop (kernel 6.1+) and a Vulkan-capable GPU with its working
system driver. GPU drivers are supplied by the operating system. This is not
a Windows executable. Preferences save to ~/.config/f1-racecraft by default.

Controls: W/S throttle/brake; A/D steer; E/Q shift; C or 1/2/3 camera;
F4 adjust camera; Esc menu; R recover; M mute. Beginner aids include steering
guidance, corner braking and automatic gears. Manual steering overrides guidance.

Build again from the repository (Python requires Pillow for texture validation):

    CARGO_BUILD_JOBS=3 python3 tools/package/build_appimage.py

The normal build supports Spa only. To include unfinished Silverstone, Bahrain
and COTA, use the experimental package command:

    CARGO_BUILD_JOBS=3 python3 tools/package/build_appimage.py --exp

This enables Cargo's `exp` feature and creates
F1-Racecraft-0.1.0-exp-linux-x86_64.AppImage. Experimental tracks are available
only in this opt-in build. BUILD.json inside the image records the included tracks.

F1_PACKAGE_ASSETS can select the converted assets directory. Build downloads
are cached under target/packaging. Official generic x86-64 Arch runtime library
packages are signature-verified; the uruntime v0.8.1 binary is SHA-256 pinned.
Runtime: https://github.com/VHSgunzo/uruntime (MIT).
Bundled package metadata, license texts and audio credits are inside the image.
Track/car and reference-audio rights remain with their respective owners.
