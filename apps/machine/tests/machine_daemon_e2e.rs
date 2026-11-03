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
            "machine_daemon_e2e{}",
            std::env::consts::EXE_SUFFIX
        ));
    let helper = std::env::var_os("MEERKAT_E2E_HELPER_MACHINE_DAEMON_E2E")
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
        String::from_utf8_lossy(&output.stdout).contains("machine-daemon e2e artifact:"),
        "daemon scenario entrypoint did not run"
    );
}

#[test]
fn local_javascript_adapter_is_loaded_without_spawning_its_cli() {
    let home = tempfile::tempdir().unwrap();
    let package = home.path().join("harnesses/custom");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("plugin.json"),
        r#"{"apiVersion":1,"id":"custom","name":"Custom","entry":"plugin.js","cli":"absent-cli"}"#,
    )
    .unwrap();
    std::fs::write(package.join("plugin.js"),r#"defineHarness({apiVersion:1,id:'custom',defaults:{environment:{WINDOW:'70'},launch_arguments:['--window','70']}})"#).unwrap();
    let config = meerkat_machine::DaemonConfig::for_home(home.path()).unwrap();
    let mut registrations = Vec::new();
    meerkat_machine::load_harness_plugins(&config, &mut registrations).unwrap();
    assert_eq!(registrations.len(), 1);
    assert_eq!(registrations[0].factory.info().harness_id, "custom");
    assert_eq!(
        registrations[0].factory.defaults().environment["WINDOW"],
        "70"
    );
    assert_eq!(
        registrations[0].probe_config.cli_path,
        std::path::Path::new("absent-cli")
    );
}
