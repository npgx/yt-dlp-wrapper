extern crate core;

mod client;
mod lock;
mod request;
mod tty;
mod video;

use clap::Parser;
use std::sync::RwLock;

pub(crate) const HEADER_VIDEO_REQUEST: &str = "67b99261-0b2c-49a9-bffd-b1f3e581f41b";
//pub(crate) const HEADER_INOUT_BIND: &str = "80476dec-f270-4462-99e7-782c6c6a2d2f";

pub(crate) static IS_VERBOSE: RwLock<bool> = RwLock::new(false);

pub(crate) const BUFFER_SIZE: usize = 1024;

pub(crate) type BoxBodyBytes = Box<
    dyn hyper::body::Body<
            Data = hyper::body::Bytes,
            Error = Box<dyn std::error::Error + Send + Sync>,
        > + Send
        + Sync
        + Unpin,
>;

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
