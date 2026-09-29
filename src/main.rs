use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Command line tool for Azure Cosmos DB")]
struct Cli {}

fn main() {
    Cli::parse();
}
