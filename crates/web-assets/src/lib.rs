//! Frontend distribution served by the Runtime composition root.
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

#[cfg(not(debug_assertions))]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../apps/web/dist/"]
struct Assets;

fn read(path: &str) -> Option<Vec<u8>> {
    #[cfg(debug_assertions)]
    {
        // Reject traversal and symlinks that leave the distribution directory.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/web/dist")
            .canonicalize()
            .ok()?;
        let file = root.join(path).canonicalize().ok()?;
        file.starts_with(&root).then_some(())?;
        std::fs::read(file).ok()
    }
    #[cfg(not(debug_assertions))]
    {
        Assets::get(path).map(|file| file.data.into_owned())
    }
}

/// Serve a distribution file or the SPA entrypoint. API routing stays in protocol.
pub fn serve(path: &str) -> Response<Full<Bytes>> {
    let path = path.trim_start_matches('/');
    if path
        .split('/')
        .any(|part| part == ".." || part.contains('\\'))
    {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::new()))
            .unwrap();
    }
    let file = read(path)
        .map(|data| (path, data))
        .or_else(|| read("index.html").map(|data| ("index.html", data)));
    let Some((name, data)) = file else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::new()))
            .unwrap();
    };
    Response::builder()
        .header(
            "content-type",
            mime_guess::from_path(name).first_or_octet_stream().as_ref(),
        )
        .header(
            "cache-control",
            if name.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            },
        )
        .header("content-length", data.len())
        .body(Full::new(Bytes::from(data)))
        .unwrap()
}
