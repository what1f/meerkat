//! Browser entrypoints on the same listener as a working Machine.
use lankide_protocol::management;
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::{path::Path, time::Duration};

#[tokio::test]
async fn a3_1_frontend_navigation_cache_and_access_control() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let url = std::fs::read_to_string(rig.runtime_run().join(management::URL_FILE))?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist");
        let index = std::fs::read(dist.join("index.html"))?;
        for path in [
            "/",
            "/members",
            "/members?tab=all",
            "/missing/deep/path",
            "/assets/does-not-exist.js",
        ] {
            let response = http.get(format!("{url}{path}")).send().await?;
            assert_eq!(response.status(), 200, "{path}");
            assert_eq!(response.headers()["cache-control"], "no-cache");
            assert_eq!(response.headers()["content-type"], "text/html");
            assert_eq!(response.bytes().await?.as_ref(), index);
        }
        let response = http.head(format!("{url}/members")).send().await?;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-length"],
            index.len().to_string()
        );
        assert!(response.bytes().await?.is_empty());
        let mut count = 0;
        for file in std::fs::read_dir(dist.join("assets"))? {
            let file = file?;
            let name = file
                .file_name()
                .to_str()
                .ok_or("invalid asset name")?
                .to_owned();
            count += 1;
            let response = http.get(format!("{url}/assets/{name}")).send().await?;
            assert_eq!(response.status(), 200);
            assert_eq!(
                response.headers()["cache-control"],
                "public, max-age=31536000, immutable"
            );
            let mime = response.headers()["content-type"].to_str()?;
            assert!(match file.path().extension().and_then(|ext| ext.to_str()) {
                Some("css") => mime == "text/css",
                Some("js") => mime == "text/javascript" || mime == "application/javascript",
                Some("png") => mime == "image/png",
                Some("svg") => mime == "image/svg+xml",
                Some("ttf") => mime == "font/ttf",
                _ => true,
            });
            assert_eq!(
                response.bytes().await?.as_ref(),
                std::fs::read(file.path())?
            );
        }
        assert!(count > 0, "build the frontend before this scenario");
        for method in [reqwest::Method::GET, reqwest::Method::POST] {
            let response = http
                .request(method, format!("{url}/api/v1/does_not_exist"))
                .json(&serde_json::json!({}))
                .send()
                .await?;
            assert_eq!(response.status(), 404);
            assert_eq!(response.headers()["content-type"], "application/json");
            let error: management::ErrorBody = response.json().await?;
            assert_eq!(error.code, management::ErrorCode::NotFound);
        }
        for path in ["/", "/members"] {
            for (header, value) in [("host", "evil.test:7440"), ("origin", "https://evil.test")] {
                let response = http
                    .get(format!("{url}{path}"))
                    .header(header, value)
                    .send()
                    .await?;
                assert_eq!(response.status(), 403);
                let error: management::ErrorBody = response.json().await?;
                assert_eq!(error.code, management::ErrorCode::PermissionDenied);
            }
        }
        // The root path still upgrades Machine links: run a real business turn.
        let team = rig.workspace("web_assets").await?;
        let agent = team
            .agent("bruce")
            .harness(Script::reply("served"))
            .create()
            .await?;
        team.direct_message(&agent, "alice", "hello").await?;
        rig.settle().await?;
        assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "served"]);
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}
