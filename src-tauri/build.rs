fn main() {
    // 应用内下载 FFmpeg 时按 Rust 目标三元组匹配清单里的平台（ffmpeg/manifest.rs）
    println!(
        "cargo:rustc-env=CLIPRECIPE_TARGET={}",
        std::env::var("TARGET").unwrap_or_default()
    );
    tauri_build::build()
}
