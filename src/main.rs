use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "version-lsp")]
#[command(version, about = "Language Server for package version management")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    // Future subcommands will be added here
    // e.g., Cache { #[command(subcommand)] action: CacheAction }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(version_lsp::lsp::server::run_server()),
    }
}
