fn main() {
    println!("cargo:rerun-if-changed=native/zb_audio_engine.cpp");
    println!("cargo:rerun-if-changed=native/zb_audio_engine.h");
    println!("cargo:rerun-if-changed=native/miniaudio_impl.cpp");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }

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
    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .include("native")
        .file("native/zb_audio_engine.cpp")
        .file("native/miniaudio_impl.cpp")
        .compile("zerobeat_audio_native");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");
}
