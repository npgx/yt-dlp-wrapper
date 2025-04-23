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
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = CliArgs::parse();
    match args.command {
        Command::Tty(args) => {
            args.yt_dlp_display.get_or_init(|| args.yt_dlp.display().to_string());
            args.beet_display.get_or_init(|| args.beet.display().to_string());
            args.fpcalc_display.get_or_init(|| args.fpcalc.display().to_string());
            args.ffmpeg_display.get_or_init(|| args.ffmpeg.display().to_string());

            tty::run(Arc::new(args)).await;
            Ok(())
        }
        Command::Request(args) => request::run(args).await,
    }
}
