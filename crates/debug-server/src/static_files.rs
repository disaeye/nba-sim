//! 本地调试服务器的静态文件服务。
//!
//! 部署目标是 Vercel 的静态目录，本地只服务 `static/` 下的文件。按请求读取磁盘，
//! 因此修改前端文件不需要重新编译调试服务器。

use std::path::{Component, Path, PathBuf};

use crate::Response;

const STATIC_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/static");

pub fn serve(request_path: &str) -> Option<Response> {
    let relative = request_path_to_relative(request_path)?;
    let path = Path::new(STATIC_ROOT).join(relative);
    if !path.is_file() {
        return None;
    }
    let body = std::fs::read(&path).ok()?;
    Some(("200 OK", content_type(&path), body))
}

fn request_path_to_relative(request_path: &str) -> Option<PathBuf> {
    let trimmed = request_path.trim_start_matches('/');
    if trimmed.is_empty() {
        return Some(PathBuf::from("index.html"));
    }
    let candidate = Path::new(trimmed);
    if candidate
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(candidate.to_path_buf())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("glb") => "model/gltf-binary",
        Some("bin") => "application/octet-stream",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("md") => "text/markdown; charset=utf-8",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_maps_to_index() {
        assert_eq!(request_path_to_relative("/"), Some(PathBuf::from("index.html")));
    }

    #[test]
    fn nested_assets_are_served() {
        assert_eq!(
            request_path_to_relative("/assets/players.glb"),
            Some(PathBuf::from("assets/players.glb"))
        );
    }

    #[test]
    fn traversal_is_rejected() {
        for path in ["/../Cargo.toml", "/assets/../../Cargo.toml", "/./index.html"] {
            assert_eq!(request_path_to_relative(path), None, "{path} 必须被拒绝");
        }
    }

    #[test]
    fn repeated_slashes_stay_below_static_root() {
        assert_eq!(
            request_path_to_relative("//etc/passwd"),
            Some(PathBuf::from("etc/passwd"))
        );
    }

    #[test]
    fn content_types_cover_static_assets() {
        assert_eq!(content_type(Path::new("app.js")), "text/javascript; charset=utf-8");
        assert_eq!(content_type(Path::new("nba_wasm_bg.wasm")), "application/wasm");
        assert_eq!(content_type(Path::new("players.glb")), "model/gltf-binary");
        assert_eq!(content_type(Path::new("unknown.bin")), "application/octet-stream");
    }
}
