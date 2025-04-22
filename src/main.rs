pub(crate) mod cli;
pub(crate) mod fingerprinting;
pub(crate) mod lock;
pub(crate) mod musicbrainz;
pub(crate) mod net;
pub(crate) mod process;
pub(crate) mod request;
pub(crate) mod signals;
pub(crate) mod tty;
pub(crate) mod user;
pub(crate) mod video;

pub(crate) mod utils;

use cli::{CliArgs, Command, Parser};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = CliArgs::parse();
    match args.command {
        Command::Tty(args) => {
            tty::run(args).await;
            Ok(())
        }
        Command::Request(args) => request::run(args).await,
    }
}
