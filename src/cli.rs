use crate::request;

pub(crate) use clap::Parser;
use std::fmt;
use std::fmt::{Display, Formatter};

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None, arg_required_else_help(true))]
pub(crate) struct CliArgs {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum Command {
    #[command(about = "Start the tty session that will handle the video requests")]
    Tty(TtyArgs),
    #[command(about = "Send a video request to the daemon")]
    Request(request::RequestArgs),
}

#[derive(clap::Args, Debug, Clone)]
pub(crate) struct TtyArgs {
    #[arg(long, visible_alias("yt-dlp-command"), value_parser = parse_yt_dlp, help = "yt-dlp command to execute. NOTE: '--' will automatically be appended to this. NOTE: each command chain will execute in a different temporary directory."
    )]
    pub(crate) yt_dlp: PosixCommand,
    #[arg(long, visible_alias("beet-command"), value_parser = parse_beet, default_value = "beet import -m", help = "beet command to execute. Defaults to 'beet import -m'. '.' will be appended to the command, and the execution directory will be set as the yt-dlp download directory."
    )]
    pub(crate) beet: PosixCommand,
    #[arg(long, default_value = "warning", help = "-loglevel to pass to ffmpeg commands")]
    pub(crate) ffmpeg_loglevel: String,
    #[arg(
        long,
        default_value = "16",
        alias = "max-request",
        help = "maximum amount of video requests that can be enqueued by request instances (this does not include the request currently being processed). Defaults to 16, will be coerced between 1 and 128 inclusive"
    )]
    pub(crate) max_requests: usize,
    #[arg(long, default_value = "never", value_parser = parse_prompt_flag, help = "Valid values: 'never', 'always', 'ask', defaults to never. It controls whether or not to keep the tmp directory where the commands are executed"
    )]
    pub(crate) keep_tmp: PromptFlag,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PromptFlag {
    Always,
    Ask,
    Never,
}

fn parse_prompt_flag(prompt: &str) -> Result<PromptFlag, TtyCliError> {
    match prompt {
        "always" => Ok(PromptFlag::Always),
        "ask" => Ok(PromptFlag::Ask),
        "never" => Ok(PromptFlag::Never),
        _ => Err(TtyCliError::InvalidKeepTmp {
            provided: prompt.to_string(),
        }),
    }
}

fn parse_yt_dlp(raw: &str) -> Result<PosixCommand, TtyCliError> {
    PosixCommand::from_raw(raw).ok_or_else(|| TtyCliError::YtDlpCommand {
        erroneous_command: raw.to_string(),
    })
}

fn parse_beet(raw: &str) -> Result<PosixCommand, TtyCliError> {
    PosixCommand::from_raw(raw).ok_or_else(|| TtyCliError::BeetCommand {
        erroneous_command: raw.to_string(),
    })
}

#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum TtyCliError {
    #[error("The provided yt-dlp command is malformed")]
    YtDlpCommand { erroneous_command: String },
    #[error("The provided beet command is malformed")]
    BeetCommand { erroneous_command: String },
    #[error("The provided keep-tmp value is invalid")]
    InvalidKeepTmp { provided: String },
}

#[derive(Debug, Clone)]
pub(crate) struct PosixCommand {
    pub(crate) components: Vec<String>,
}

impl PosixCommand {
    fn new(args: Vec<String>) -> Self {
        Self { components: args }
    }

    pub(crate) fn from_raw(raw: &str) -> Option<Self> {
        shlex::split(raw).map(Self::new)
    }
}

impl Display for PosixCommand {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
