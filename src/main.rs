mod lock;
mod request;
mod tty;
mod video;

pub(crate) mod tools;

use clap::Parser;

#[macro_export]
macro_rules! verbose {
    ($($arg:tt)*) => {
        if let Ok(guard) = crate::IS_VERBOSE.read() {
            if *guard {
                println!($($arg)*);
            }
        }
    };
}

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None, arg_required_else_help(true))]
pub(crate) struct CliArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum Command {
    #[command(about = "Start the tty session that will handle the video requests")]
    Tty(tty::TtyArgs),
    #[command(about = "Send a video request to the daemon")]
    Request(request::RequestArgs),
}

#[tokio::main]
async fn main() {
    let args = CliArgs::parse();
    match args.command {
        Command::Tty(args) => {
            tty::run(args).await;
        }
        Command::Request(args) => {
            request::run(args).await;
        }
    }
}
