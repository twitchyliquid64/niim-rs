use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser)]
#[command(name = "niimctl", version)]
#[command(about = "Print labels on NIIMBOT label makers")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate shell completion script
    #[command(
        long_about = "Generate a shell tab-completion script for niimctl.\nSupported shells include bash, zsh, elvish and fish.\n\n   source <(niimctl completions bash)"
    )]
    Completions(CompletionsArgs),
}

#[derive(Args)]
struct CompletionsArgs {
    /// The shell to generate completions for
    shell: Shell,
}

#[tokio::main]
async fn main() {
    match Cli::parse().command {
        Command::Completions(args) => {
            clap_complete::generate(
                args.shell,
                &mut Cli::command(),
                "niimctl",
                &mut std::io::stdout(),
            );
        }
    }
}
