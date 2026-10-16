#[test]
fn daemon_business_scenario() {
    let executable = std::env::current_exe().expect("test executable");
    let helper = executable
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join(format!(
            "runtime_daemon_e2e{}",
            std::env::consts::EXE_SUFFIX
        ));
    let helper = std::env::var_os("LANKIDE_E2E_HELPER_RUNTIME_DAEMON_E2E")
        .map(std::path::PathBuf::from)
        .unwrap_or(helper);
    let output = std::process::Command::new(helper)
        .output()
        .expect("start daemon scenario helper");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("runtime-daemon E2E passed; result="),
        "daemon scenario entrypoint did not run"
    );
}
