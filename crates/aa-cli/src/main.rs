//! `aa` binary entry point.

#[tokio::main]
async fn main() {
    std::process::exit(aa_cli::run(std::env::args_os().collect()).await);
}
