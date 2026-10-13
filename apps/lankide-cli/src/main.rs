#![forbid(unsafe_code)]

#[tokio::main]
async fn main() {
    match lankide_cli::execute_from_environment(std::env::args().skip(1)).await {
        Ok(output) => println!(
            "{}",
            serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_owned())
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}
