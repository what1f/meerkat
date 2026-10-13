#![forbid(unsafe_code)]

mod daemon_e2e;
use daemon_e2e::run_e2e;
use lankide_runtime_daemon::{RuntimePaths, run_daemon, wait_for_shutdown};

#[tokio::main]
async fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.iter().any(|argument| argument == "--e2e") {
        match tokio::time::timeout(std::time::Duration::from_secs(40), run_e2e()).await {
            Ok(Ok(artifact)) => {
                println!("runtime-daemon E2E passed; result={}", artifact.display());
                return;
            }
            Ok(Err(error)) => eprintln!("runtime-daemon E2E failed: {error}"),
            Err(_) => eprintln!("runtime-daemon E2E exceeded its hard timeout"),
        }
        std::process::exit(1);
    }

    let home = arguments
        .windows(2)
        .find(|pair| pair[0] == "--home")
        .map(|pair| std::path::PathBuf::from(&pair[1]));
    let fixture = arguments
        .iter()
        .any(|argument| argument == "--e2e-fixture-provider");
    let paths = match home {
        Some(home) => RuntimePaths::for_home(home),
        None => RuntimePaths::local_default(),
    };
    let result = match paths {
        Ok(paths) => {
            run_daemon(
                paths,
                if fixture {
                    daemon_e2e::fixture_factories().expect("fixture factories")
                } else {
                    lankide_runtime_daemon::production_factories().expect("production factories")
                },
                wait_for_shutdown(),
            )
            .await
        }
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("runtime-daemon could not start: {error}");
        std::process::exit(1);
    }
}
