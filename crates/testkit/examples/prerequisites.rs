use meerkat_testkit::{GateSelection, ProbeConfig, run_prerequisite_probe};

fn main() {
    let mut format = "text";
    let mut gate = GateSelection::All;
    let mut run_live_checks = true;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--format" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    usage_and_exit();
                };
                format = value;
                if !matches!(format, "text" | "json") {
                    usage_and_exit();
                }
            }
            "--gate" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    usage_and_exit();
                };
                gate = match value.as_str() {
                    "all" => GateSelection::All,
                    "hermetic" => GateSelection::Hermetic,
                    "feishu-live" => GateSelection::FeishuLive,
                    "telegram-live-readonly" => GateSelection::TelegramLiveReadonly,
                    "telegram-live-mutating" => GateSelection::TelegramLiveMutating,
                    "harness-live" => GateSelection::HarnessLive,
                    _ => usage_and_exit(),
                };
            }
            "--no-live" => run_live_checks = false,
            "--help" | "-h" => usage_and_exit_with_code(0),
            _ => usage_and_exit(),
        }
        index += 1;
    }

    let report = run_prerequisite_probe(&ProbeConfig::from_environment(gate, run_live_checks));
    let rendered = if format == "json" {
        report
            .render_json()
            .expect("the prerequisite report must be serializable")
    } else {
        report.render_text()
    };
    print!("{rendered}");
    if !report.is_green() {
        std::process::exit(1);
    }
}

fn usage_and_exit() -> ! {
    usage_and_exit_with_code(2)
}

fn usage_and_exit_with_code(code: i32) -> ! {
    eprintln!(
        "usage: cargo run -p meerkat-testkit --example prerequisites -- [--format text|json] [--gate all|hermetic|feishu-live|telegram-live-readonly|telegram-live-mutating|harness-live] [--no-live]"
    );
    std::process::exit(code)
}
