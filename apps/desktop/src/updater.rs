//! Keeps an installed Meerkat bundle current with the latest GitHub release.
//!
//! A newer release is downloaded and verified next to the running bundle, then
//! swapped in when the user restarts or quits. Both bundled helpers are
//! restarted right after the swap so they never outlive the old app.

use serde::Deserialize;
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{AppHandle, Manager, Url, menu::MenuItem};

pub(crate) const MENU_ID: &str = "check-update";
const LATEST_RELEASE: &str = "https://api.github.com/repos/what1f/meerkat/releases/latest";
/// Points the updater at another release feed, e.g. a staging mirror.
const FEED_ENV: &str = "MEERKAT_UPDATE_FEED";
#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "com.meerkat.desktop";
#[cfg(target_os = "macos")]
const STAGING_DIR: &str = ".Meerkat-update";
#[cfg(target_os = "macos")]
const PREVIOUS_BUNDLE: &str = ".Meerkat-previous.app";
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub(crate) struct Updater {
    chinese: bool,
    item: MenuItem<tauri::Wry>,
    busy: AtomicBool,
    phase: Mutex<Phase>,
}

enum Phase {
    Idle,
    /// A newer release this copy cannot replace in place.
    Available {
        #[cfg(target_os = "macos")]
        version: String,
        page: String,
        #[cfg(target_os = "macos")]
        prompted: bool,
    },
    #[cfg(target_os = "macos")]
    Ready {
        version: String,
        staged: PathBuf,
        prompted: bool,
    },
    #[cfg(target_os = "macos")]
    Applied,
}

enum Found {
    Page {
        version: String,
        page: String,
    },
    #[cfg(target_os = "macos")]
    Staged {
        version: String,
        staged: PathBuf,
    },
}

impl Updater {
    pub(crate) fn new(chinese: bool, item: MenuItem<tauri::Wry>) -> Self {
        Self {
            chinese,
            item,
            busy: AtomicBool::new(false),
            phase: Mutex::new(Phase::Idle),
        }
    }

    pub(crate) fn menu_text(chinese: bool) -> &'static str {
        if chinese {
            "检查更新…"
        } else {
            "Check for Updates…"
        }
    }

    #[cfg(target_os = "macos")]
    fn text(&self, zh: &'static str, en: &'static str) -> &'static str {
        if self.chinese { zh } else { en }
    }

    fn show_idle(&self) {
        let _ = self.item.set_text(Self::menu_text(self.chinese));
        let _ = self.item.set_enabled(true);
    }

    fn show_busy(&self) {
        let _ = self.item.set_enabled(false);
    }

    #[cfg(target_os = "macos")]
    fn show_ready(&self) {
        let _ = self
            .item
            .set_text(self.text("重启以完成更新", "Restart to Update"));
        let _ = self.item.set_enabled(true);
    }

    #[cfg(not(target_os = "macos"))]
    fn show_available(&self, version: &str) {
        let label = if self.chinese {
            format!("下载 Meerkat {version}…")
        } else {
            format!("Download Meerkat {version}…")
        };
        let _ = self.item.set_text(label);
        let _ = self.item.set_enabled(true);
    }

    #[cfg(not(target_os = "macos"))]
    fn show_transient_status(&self, zh: &'static str, en: &'static str) {
        let _ = self.item.set_text(if self.chinese { zh } else { en });
        let _ = self.item.set_enabled(true);
    }
}

/// Starts periodic checks for an installed bundle; development runs stay quiet.
pub(crate) fn start(app: &AppHandle) {
    if !should_check_updates() {
        return;
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("meerkat-updater".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            loop {
                check(&app, false);
                std::thread::sleep(CHECK_INTERVAL);
            }
        })
        .expect("failed to start updater thread");
}

pub(crate) fn menu_clicked(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let ready = matches!(
            *app.state::<Updater>().phase.lock().unwrap(),
            Phase::Ready { .. }
        );
        if ready {
            restart(app);
            return;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let page = match &*app.state::<Updater>().phase.lock().unwrap() {
            Phase::Available { page, .. } => Some(page.clone()),
            _ => None,
        };
        if let Some(page) = page
            && let Ok(url) = Url::parse(&page)
        {
            crate::open_external(app, &url);
            return;
        }
    }
    let app = app.clone();
    std::thread::spawn(move || check(&app, true));
}

/// Offers a found update once the user is back in the window.
pub(crate) fn window_focused(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let pending = match &*app.state::<Updater>().phase.lock().unwrap() {
            Phase::Ready { prompted, .. } | Phase::Available { prompted, .. } => !prompted,
            _ => false,
        };
        if pending {
            offer(app);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// Installs a downloaded update when the user quits without restarting.
pub(crate) fn install_on_exit(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    if let Err(error) = install(app) {
        eprintln!("meerkat-desktop: could not install update: {error}");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

fn check(app: &AppHandle, manual: bool) {
    let updater = app.state::<Updater>();
    #[cfg(target_os = "macos")]
    let already_done = matches!(
        *updater.phase.lock().unwrap(),
        Phase::Ready { .. } | Phase::Applied
    );
    #[cfg(not(target_os = "macos"))]
    let already_done = false;
    if already_done || updater.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    updater.show_busy();
    let outcome = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
        .and_then(|runtime| runtime.block_on(find_and_stage()));
    updater.busy.store(false, Ordering::SeqCst);

    match outcome {
        Ok(None) => {
            *updater.phase.lock().unwrap() = Phase::Idle;
            #[cfg(target_os = "macos")]
            {
                updater.show_idle();
                if manual {
                    let title = updater.text("Meerkat 已是最新版本", "Meerkat is up to date");
                    let ok = updater.text("好", "OK");
                    on_main(app, move |_| {
                        alert(title, None, &[ok]);
                    });
                }
            }
            #[cfg(not(target_os = "macos"))]
            if manual {
                updater.show_transient_status("已是最新版本", "Up to date");
                let app = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(4));
                    let updater = app.state::<Updater>();
                    if matches!(*updater.phase.lock().unwrap(), Phase::Idle) {
                        updater.show_idle();
                    }
                });
            } else {
                updater.show_idle();
            }
        }
        #[cfg(target_os = "macos")]
        Ok(Some(Found::Staged { version, staged })) => {
            *updater.phase.lock().unwrap() = Phase::Ready {
                version,
                staged,
                prompted: false,
            };
            updater.show_ready();
            if manual || window_is_focused(app) {
                offer(app);
            }
        }
        Ok(Some(Found::Page { version, page })) => {
            #[cfg(target_os = "macos")]
            {
                updater.show_idle();
                let mut phase = updater.phase.lock().unwrap();
                let seen = matches!(&*phase, Phase::Available { version: known, prompted: true, .. } if *known == version);
                *phase = Phase::Available {
                    version,
                    page,
                    prompted: seen && !manual,
                };
                drop(phase);
                if manual || (!seen && window_is_focused(app)) {
                    offer(app);
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                updater.show_available(&version);
                *updater.phase.lock().unwrap() = Phase::Available { page: page.clone() };
                if manual && let Ok(url) = Url::parse(&page) {
                    crate::open_external(app, &url);
                }
            }
        }
        Err(error) => {
            eprintln!("meerkat-desktop: update check failed: {error}");
            #[cfg(target_os = "macos")]
            {
                updater.show_idle();
                if manual {
                    let title = updater.text("暂时无法检查更新", "Couldn't check for updates");
                    let ok = updater.text("好", "OK");
                    on_main(app, move |_| {
                        alert(title, None, &[ok]);
                    });
                }
            }
            #[cfg(not(target_os = "macos"))]
            if manual {
                updater.show_transient_status("暂时无法检查更新", "Couldn't check for updates");
                let app = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(4));
                    let updater = app.state::<Updater>();
                    if matches!(*updater.phase.lock().unwrap(), Phase::Idle) {
                        updater.show_idle();
                    }
                });
            } else {
                updater.show_idle();
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn offer(app: &AppHandle) {
    on_main(app, |app| {
        let updater = app.state::<Updater>();
        let mut phase = updater.phase.lock().unwrap();
        match &mut *phase {
            Phase::Ready {
                version, prompted, ..
            } if !*prompted => {
                *prompted = true;
                let title = if updater.chinese {
                    format!("Meerkat {version} 已下载")
                } else {
                    format!("Meerkat {version} is ready")
                };
                drop(phase);
                let restart_now = alert(
                    &title,
                    Some(updater.text(
                        "稍后会在退出时自动更新。",
                        "Otherwise it installs when you quit.",
                    )),
                    &[
                        updater.text("立即重启", "Restart Now"),
                        updater.text("稍后", "Later"),
                    ],
                ) == 0;
                if restart_now {
                    restart(app);
                }
            }
            Phase::Available {
                version,
                page,
                prompted,
            } if !*prompted => {
                *prompted = true;
                let title = if updater.chinese {
                    format!("Meerkat {version} 已发布")
                } else {
                    format!("Meerkat {version} is available")
                };
                let page = page.clone();
                drop(phase);
                let download = alert(
                    &title,
                    None,
                    &[
                        updater.text("前往下载", "Download"),
                        updater.text("稍后", "Later"),
                    ],
                ) == 0;
                if download && let Ok(url) = Url::parse(&page) {
                    crate::open_external(app, &url);
                }
            }
            _ => {}
        }
    });
}

#[cfg(target_os = "macos")]
fn restart(app: &AppHandle) {
    match install(app) {
        Ok(Some(bundle)) => {
            relaunch(&bundle);
            app.exit(0);
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("meerkat-desktop: could not install update: {error}");
            let updater = app.state::<Updater>();
            updater.show_idle();
            let title = updater.text("更新没有完成", "Couldn't finish the update");
            let ok = updater.text("好", "OK");
            on_main(app, move |_| {
                alert(title, None, &[ok]);
            });
        }
    }
}

/// Swaps the staged bundle in and restarts both helpers from it.
#[cfg(target_os = "macos")]
fn install(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let updater = app.state::<Updater>();
    let staged = {
        let mut phase = updater.phase.lock().unwrap();
        if !matches!(*phase, Phase::Ready { .. }) {
            return Ok(None);
        }
        match std::mem::replace(&mut *phase, Phase::Applied) {
            Phase::Ready { staged, .. } => staged,
            _ => unreachable!(),
        }
    };
    let swapped = swap(&staged);
    if let Some(staging) = staged.parent() {
        let _ = fs::remove_dir_all(staging);
    }
    match swapped {
        Ok(bundle) => {
            crate::background_services::restart_bundled_helpers(&bundle);
            Ok(Some(bundle))
        }
        Err(error) => {
            *updater.phase.lock().unwrap() = Phase::Idle;
            Err(error)
        }
    }
}

#[cfg(target_os = "macos")]
fn swap(staged: &Path) -> Result<PathBuf, String> {
    let bundle = current_bundle().ok_or("not running from an installed bundle")?;
    let parent = bundle.parent().ok_or("bundle has no parent")?;
    let previous = parent.join(PREVIOUS_BUNDLE);
    let _ = fs::remove_dir_all(&previous);
    fs::rename(&bundle, &previous).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(staged, &bundle) {
        let _ = fs::rename(&previous, &bundle);
        return Err(error.to_string());
    }
    let _ = fs::remove_dir_all(&previous);
    Ok(bundle)
}

/// Opens the new bundle once this process has exited.
#[cfg(target_os = "macos")]
fn relaunch(bundle: &Path) {
    use std::os::unix::process::CommandExt as _;
    let script =
        r#"while /bin/kill -0 "$0" 2>/dev/null; do /bin/sleep 0.2; done; exec /usr/bin/open "$1""#;
    if let Err(error) = Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg(std::process::id().to_string())
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
    {
        eprintln!("meerkat-desktop: could not schedule relaunch: {error}");
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

impl Release {
    fn asset(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.as_str())
    }
}

async fn find_and_stage() -> Result<Option<Found>, String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("Meerkat/{CURRENT}"))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(15 * 60))
        .build()
        .map_err(|error| error.to_string())?;
    let feed = std::env::var(FEED_ENV).unwrap_or_else(|_| LATEST_RELEASE.into());
    let release: Release = client
        .get(feed)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())?;
    let version = release.tag_name.trim_start_matches('v').to_owned();
    if !is_newer(&version, CURRENT) {
        return Ok(None);
    }

    let architecture = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    #[cfg(target_os = "macos")]
    {
        let page = Found::Page {
            version: version.clone(),
            page: release.html_url.clone(),
        };
        let archive_name = format!("Meerkat-{version}-macos-{architecture}.zip");
        let checksum_name = format!("Meerkat-{version}-macos-{architecture}.sha256");
        let (Some(archive_url), Some(checksum_url), Some(bundle)) = (
            release.asset(&archive_name),
            release.asset(&checksum_name),
            current_bundle(),
        ) else {
            return Ok(Some(page));
        };
        // Staging beside the bundle proves it is writable and keeps the swap a rename.
        let Some(staging) = bundle.parent().map(|parent| parent.join(STAGING_DIR)) else {
            return Ok(Some(page));
        };
        let _ = fs::remove_dir_all(&staging);
        if fs::create_dir(&staging).is_err() {
            return Ok(Some(page));
        }

        let staged = stage(
            &client,
            archive_url,
            &archive_name,
            checksum_url,
            &staging,
            &version,
            &bundle,
        )
        .await;
        if staged.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        staged.map(|staged| Some(Found::Staged { version, staged }))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let download_url = match std::env::consts::OS {
            "windows" => release
                .asset(&format!(
                    "Meerkat-{version}-windows-{architecture}-setup.exe"
                ))
                .or_else(|| {
                    release.asset(&format!("Meerkat-{version}-windows-{architecture}.zip"))
                }),
            "linux" => release.asset(&format!("Meerkat-{version}-linux-{architecture}.tar.gz")),
            _ => None,
        }
        .map(str::to_owned)
        .unwrap_or_else(|| release.html_url.clone());
        Ok(Some(Found::Page {
            version,
            page: download_url,
        }))
    }
}

#[cfg(target_os = "macos")]
async fn stage(
    client: &reqwest::Client,
    archive_url: &str,
    archive_name: &str,
    checksum_url: &str,
    staging: &Path,
    version: &str,
    bundle: &Path,
) -> Result<PathBuf, String> {
    let checksums = client
        .get(checksum_url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    let expected = checksums
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim().trim_start_matches('*') == archive_name)
        .map(|(hash, _)| hash.to_ascii_lowercase())
        .ok_or("release checksum is missing")?;

    let archive = staging.join(archive_name);
    let mut response = client
        .get(archive_url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.to_string())?;
    let mut file = fs::File::create(&archive).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        hasher.update(&chunk);
        file.write_all(&chunk).map_err(|error| error.to_string())?;
    }
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    let actual = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected {
        return Err("downloaded archive does not match its checksum".into());
    }

    run(Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(&archive)
        .arg(staging))?;
    let _ = fs::remove_file(&archive);
    let staged = staging.join("Meerkat.app");
    verify(&staged, version, bundle)?;
    Ok(staged)
}

#[cfg(target_os = "macos")]
fn verify(staged: &Path, version: &str, current: &Path) -> Result<(), String> {
    run(Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(staged))?;
    let info = staged.join("Contents/Info.plist");
    if plist_value(&info, "CFBundleIdentifier")? != BUNDLE_ID
        || plist_value(&info, "CFBundleShortVersionString")? != version
    {
        return Err("downloaded bundle is not the expected release".into());
    }
    // A Developer ID build only accepts updates signed by the same team.
    if let Some(team) = signing_team(current)
        && signing_team(staged).as_deref() != Some(team.as_str())
    {
        return Err("downloaded bundle is signed by another team".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn plist_value(plist: &Path, key: &str) -> Result<String, String> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(plist)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("{key} is missing from the downloaded bundle"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(target_os = "macos")]
fn signing_team(bundle: &Path) -> Option<String> {
    let output = Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=2"])
        .arg(bundle)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|team| !team.is_empty() && *team != "not set")
        .map(str::to_owned)
}

#[cfg(target_os = "macos")]
fn run(command: &mut Command) -> Result<(), String> {
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |value: &str| {
        value
            .split('.')
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
    };
    match (parse(candidate), parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn should_check_updates() -> bool {
    #[cfg(target_os = "macos")]
    {
        current_bundle().is_some()
    }
    #[cfg(not(target_os = "macos"))]
    {
        if std::env::var_os(FEED_ENV).is_some() {
            return true;
        }
        std::env::current_exe().ok().is_some_and(|path| {
            !path
                .components()
                .any(|component| component.as_os_str() == "target")
        })
    }
}

/// The installed `.app` this process runs from, if it can be replaced.
#[cfg(target_os = "macos")]
fn current_bundle() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let bundle = executable.parent()?.parent()?.parent()?;
    let is_bundle = bundle
        .extension()
        .is_some_and(|extension| extension == "app")
        && executable.parent()?.ends_with("Contents/MacOS");
    let translocated = bundle
        .components()
        .any(|component| component.as_os_str() == "AppTranslocation");
    (is_bundle && !translocated).then(|| bundle.to_path_buf())
}

#[cfg(target_os = "macos")]
fn window_is_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|window| window.is_focused().ok())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn on_main(app: &AppHandle, task: impl FnOnce(&AppHandle) + Send + 'static) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || task(&handle)) {
        eprintln!("meerkat-desktop: could not show update prompt: {error}");
    }
}

/// Shows an app-modal alert and returns the index of the chosen button.
#[cfg(target_os = "macos")]
fn alert(title: &str, detail: Option<&str>, buttons: &[&str]) -> usize {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn};
    use objc2_foundation::NSString;

    let marker = MainThreadMarker::new().expect("update prompt on main thread");
    let alert = NSAlert::new(marker);
    alert.setMessageText(&NSString::from_str(title));
    if let Some(detail) = detail {
        alert.setInformativeText(&NSString::from_str(detail));
    }
    for button in buttons {
        alert.addButtonWithTitle(&NSString::from_str(button));
    }
    let response = alert.runModal();
    usize::try_from(response - NSAlertFirstButtonReturn).unwrap_or(usize::MAX)
}
