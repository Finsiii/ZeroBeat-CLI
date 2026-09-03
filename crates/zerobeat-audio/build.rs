#[path = "src/build_config.rs"]
mod build_config;

use std::{env, path::PathBuf};

use build_config::{
    NativePlatform, classify_platform, has_windows_dependencies, split_path_list, windows_triplet,
};

fn main() {
    println!("cargo:rerun-if-changed=native/zb_audio_engine.cpp");
    println!("cargo:rerun-if-changed=native/zb_audio_engine.h");
    println!("cargo:rerun-if-changed=native/miniaudio_impl.cpp");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match classify_platform(&target_os) {
        NativePlatform::Linux => build_linux(),
        NativePlatform::Windows => build_windows(),
        NativePlatform::Unsupported => {}
    }
}

fn build_linux() {
    for library in [
        "libavformat",
        "libavcodec",
        "libavutil",
        "libswresample",
        "libcurl",
    ] {
        pkg_config::Config::new()
            .probe(library)
            .unwrap_or_else(|error| panic!("ZeroBeat native audio requires {library}: {error}"));
    }
    compile_native(&[], &[]);
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");
}

fn build_windows() {
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let default_triplet = windows_triplet(&target_arch, &target_env).unwrap_or_else(|| {
        panic!(
            "ZeroBeat native Windows audio supports only x86_64-pc-windows-msvc (got {target_arch}-{target_env})"
        )
    });

    for name in [
        "VCPKG_ROOT",
        "VCPKG_INSTALLATION_ROOT",
        "VCPKG_INSTALLED_DIR",
        "VCPKG_TARGET_TRIPLET",
        "ZEROBEAT_FFMPEG_DIR",
        "FFMPEG_DIR",
        "ZEROBEAT_CURL_DIR",
        "CURL_DIR",
        "ZEROBEAT_NATIVE_INCLUDE_DIRS",
        "ZEROBEAT_NATIVE_LIB_DIRS",
        "FFMPEG_INCLUDE_DIR",
        "FFMPEG_LIB_DIR",
        "CURL_INCLUDE_DIR",
        "CURL_LIB_DIR",
        "INCLUDE",
        "LIB",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let triplet = env::var("VCPKG_TARGET_TRIPLET").unwrap_or_else(|_| default_triplet.to_owned());
    if triplet != "x64-windows" {
        panic!("ZeroBeat native Windows audio requires dynamic vcpkg triplet x64-windows");
    }
    let (include_dirs, lib_dirs) = windows_dependency_dirs(&triplet);
    if !has_windows_dependencies(&include_dirs, &lib_dirs) {
        panic!(
            "ZeroBeat native Windows audio could not find FFmpeg and libcurl headers/import libraries; set VCPKG_ROOT, VCPKG_INSTALLED_DIR, or explicit include/lib directories"
        );
    }
    compile_native(&include_dirs, &lib_dirs);
    for library in ["avformat", "avcodec", "avutil", "swresample", "libcurl"] {
        println!("cargo:rustc-link-lib={library}");
    }
}

fn compile_native(include_dirs: &[PathBuf], lib_dirs: &[PathBuf]) {
    let mut build = cc::Build::new();
    build.cpp(true).std("c++20").include("native");
    for directory in include_dirs {
        build.include(directory);
    }
    build
        .file("native/zb_audio_engine.cpp")
        .file("native/miniaudio_impl.cpp")
        .compile("zerobeat_audio_native");
    for directory in lib_dirs {
        println!("cargo:rustc-link-search=native={}", directory.display());
    }
}

fn windows_dependency_dirs(triplet: &str) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut include_dirs = Vec::new();
    let mut lib_dirs = Vec::new();
    for variable in ["ZEROBEAT_NATIVE_INCLUDE_DIRS", "INCLUDE"] {
        append_path_list(&mut include_dirs, variable);
    }
    for variable in ["ZEROBEAT_NATIVE_LIB_DIRS", "LIB"] {
        append_path_list(&mut lib_dirs, variable);
    }
    for variable in [
        "ZEROBEAT_FFMPEG_DIR",
        "FFMPEG_DIR",
        "ZEROBEAT_CURL_DIR",
        "CURL_DIR",
    ] {
        if let Some(root) = env::var_os(variable) {
            append_root(&mut include_dirs, &mut lib_dirs, PathBuf::from(root));
        }
    }
    for variable in ["FFMPEG_INCLUDE_DIR", "CURL_INCLUDE_DIR"] {
        append_path_list(&mut include_dirs, variable);
    }
    for variable in ["FFMPEG_LIB_DIR", "CURL_LIB_DIR"] {
        append_path_list(&mut lib_dirs, variable);
    }
    if let Some(installed) = env::var_os("VCPKG_INSTALLED_DIR") {
        append_root(
            &mut include_dirs,
            &mut lib_dirs,
            PathBuf::from(installed).join(triplet),
        );
    } else if let Some(root) =
        env::var_os("VCPKG_ROOT").or_else(|| env::var_os("VCPKG_INSTALLATION_ROOT"))
    {
        let installed = PathBuf::from(root).join("installed");
        append_root(&mut include_dirs, &mut lib_dirs, installed.join(triplet));
    }
    (include_dirs, lib_dirs)
}

fn append_path_list(paths: &mut Vec<PathBuf>, variable: &str) {
    if let Some(value) = env::var_os(variable) {
        for path in split_path_list(&value.to_string_lossy()) {
            let path = PathBuf::from(path);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
}

fn append_root(include_dirs: &mut Vec<PathBuf>, lib_dirs: &mut Vec<PathBuf>, root: PathBuf) {
    let include = root.join("include");
    let lib = root.join("lib");
    if !include_dirs.contains(&include) {
        include_dirs.push(include);
    }
    if !lib_dirs.contains(&lib) {
        lib_dirs.push(lib);
    }
}
