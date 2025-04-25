use anyhow::anyhow;
pub(crate) use clap::Parser;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None, arg_required_else_help(true))]
pub(crate) struct CliArgs {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[allow(clippy::large_enum_variant)]
#[derive(clap::Subcommand, Debug)]
pub(crate) enum Command {
    #[command(about = "Start the tty session that will handle the video requests")]
    Tty(TtyArgs),
    #[command(about = "Send a video request to the tty instance")]
    Request(RequestArgs),
}

mod tty_about {
    pub(super) const YT_DLP_EXEC: &str = "'yt-dlp' executable location.";
    pub(super) const YT_DLP_ARGS: &str = "Extra arguments to pass to '--yt-dlp'. NOTE: '--' will automatically be appended to this. NOTE: each command chain will execute in a different temporary directory.";

    pub(super) const BEET_EXEC: &str = "'beet' executable location.";
    pub(super) const BEET_ARGS: &str = "Extra arguments to pass to '--beet'. '.' will be appended to the command, and the execution directory will be set as the /tmp directory where yt-dlp was executed.";

    pub(super) const FPCALC_EXEC: &str = "'fpcalc' executable location.";

    pub(super) const FFMPEG_EXEC: &str = "'ffmpeg' executable location.";
    pub(super) const FFMPEG_LOGLEVEL: &str = "'-loglevel' argument for ffmpeg commands";

    pub(super) const MAX_REQUESTS: &str = "Maximum amount of video requests that can be enqueued by request instances (this does not include the request currently being processed). Clamped between 1 and 256, inclusive.";

    pub(super) const KEEP_TMP: &str =
        "It controls whether or not to keep the tmp directory where the commands are executed.";

    pub(super) const PORT_OVERRIDE: &str = "Use <PORT_OVERRIDE> as the http server's port, instead of using the default behaviour which is to use an OS-provided random port.";

    pub(super) const LOCK_OVERRIDE: &str =
        "ONLY ENABLE THIS IF YOU KNOW WHAT YOU ARE DOING. Enabling this will disable the lockfile and the portfile.";
}

#[derive(clap::Args, Debug, Clone)]
pub(crate) struct TtyArgs {
    #[arg(long, visible_alias("yt-dlp-executable"), default_value = "yt-dlp", help = tty_about::YT_DLP_EXEC)]
    pub(crate) yt_dlp: PathBuf,
    #[arg(skip)]
    pub(crate) yt_dlp_display: once_cell::sync::OnceCell<String>,
    #[arg(long, value_parser = parse_yt_dlp_args, default_value = "", allow_hyphen_values = true, help = tty_about::YT_DLP_ARGS)]
    pub(crate) yt_dlp_args: PosixSplit,
    #[arg(long, visible_alias("beet-executable"), default_value = "beet", help = tty_about::BEET_EXEC)]
    pub(crate) beet: PathBuf,
    #[arg(skip)]
    pub(crate) beet_display: once_cell::sync::OnceCell<String>,
    #[arg(long, value_parser = parse_beet_args, default_value = "import -m -s", allow_hyphen_values = true, help = tty_about::BEET_ARGS)]
    pub(crate) beet_args: PosixSplit,
    #[arg(long, visible_alias("fpcalc-executable"), default_value = "fpcalc", help = tty_about::FPCALC_EXEC)]
    pub(crate) fpcalc: PathBuf,
    #[arg(skip)]
    pub(crate) fpcalc_display: once_cell::sync::OnceCell<String>,
    #[arg(long, visible_alias("ffmpeg-executable"), default_value = "ffmpeg", help = tty_about::FFMPEG_EXEC)]
    pub(crate) ffmpeg: PathBuf,
    #[arg(skip)]
    pub(crate) ffmpeg_display: once_cell::sync::OnceCell<String>,
    #[arg(long, default_value = "warning", help = tty_about::FFMPEG_LOGLEVEL)]
    pub(crate) ffmpeg_loglevel: String,
    #[arg(long, default_value = "16", alias = "max-request", help = tty_about::MAX_REQUESTS)]
    pub(crate) max_requests: u32,
    #[arg(long, default_value = "never", value_parser = parse_prompt_flag, value_name = "always/ask/never", help = tty_about::KEEP_TMP)]
    pub(crate) keep_tmp: PromptFlag,
    #[arg(long, help = tty_about::PORT_OVERRIDE)]
    pub(crate) port_override: Option<u16>,
    #[arg(long, help = tty_about::LOCK_OVERRIDE)]
    pub(crate) dangerously_skip_lock_checks: bool,
}

mod request_about {
    pub(super) const YT_URL: &str = "Youtube url to use for creating the video request. Supports the majority of modern youtube urls (will extract the ID).";

    pub(super) const PORT_OVERRIDE: &str =
        "Manually specify the tty instance's http port instead of reading from the lockfile.";

    pub(super) const LOCK_OVERRIDE: &str = "ONLY ENABLE THIS IF YOU KNOW WHAT YOU ARE DOING. Usually request instances will try to tell if the tty instance is running though the lockfile's ownership. This disables that. If this option is active, you NEED to specify a port manually.";
}

#[derive(clap::Args, Debug)]
pub(crate) struct RequestArgs {
    #[arg(long, help = request_about::YT_URL)]
    pub(crate) yt_url: String,
    #[arg(long, visible_alias("http_port"), help = request_about::PORT_OVERRIDE)]
    pub(crate) port_override: Option<u16>,
    #[arg(long, help = request_about::LOCK_OVERRIDE)]
    pub(crate) dangerously_skip_lock_checks: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PromptFlag {
    Always,
    Ask,
    Never,
}

fn parse_prompt_flag(prompt: &str) -> Result<PromptFlag, anyhow::Error> {
    match prompt.to_lowercase().as_str() {
        "always" => Ok(PromptFlag::Always),
        "ask" => Ok(PromptFlag::Ask),
        "never" => Ok(PromptFlag::Never),
        _ => Err(anyhow!(
            "Invalid value: '{}', allowed values are 'always', 'ask' and 'never'",
            prompt
        )),
    }
}

fn parse_yt_dlp_args(args: &str) -> Result<PosixSplit, anyhow::Error> {
    PosixSplit::from_raw(args).ok_or_else(|| anyhow!("Couldn't parse argument: --yt-dlp-args"))
}

fn parse_beet_args(args: &str) -> Result<PosixSplit, anyhow::Error> {
    PosixSplit::from_raw(args).ok_or_else(|| anyhow!("Couldn't parse argument: --beet-args"))
}

#[derive(Debug, Clone)]
pub(crate) struct PosixSplit {
    pub(crate) components: Vec<String>,
}

impl PosixSplit {
    fn new(args: Vec<String>) -> Self {
        Self { components: args }
    }

    pub(crate) fn from_raw(raw: &str) -> Option<Self> {
        shlex::split(raw).map(Self::new)
    }
}

impl Display for PosixSplit {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
