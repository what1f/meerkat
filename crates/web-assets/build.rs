use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    if std::env::var("PROFILE").as_deref() == Ok("release") && !dist.join("index.html").is_file() {
        panic!("Web frontend is missing. Run `pnpm -C apps/web build` before the release build.");
    }
}
