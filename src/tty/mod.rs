pub(crate) mod handle_requests;

use crate::lock;
use crate::tty::handle_requests::ExitStatusExt;
use crate::video::VideoRequest;
use axum::response::Response;
use axum::routing::post;
use clap::arg;
use console::style;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc::error::TrySendError;
use tower::ServiceBuilder;

#[derive(clap::Args, Debug, Clone)]
pub(crate) struct TtyArgs {
    #[arg(long, visible_alias("yt-dlp-command"), value_parser = parse_yt_dlp, help = "yt-dlp command to execute. NOTE: '--' will automatically be appended to this. NOTE: each command chain will execute in a different temporary directory.")]
    pub yt_dlp: PosixCommand,
    #[arg(long, visible_alias("beet-command"), value_parser = parse_beet, default_value = "beet import -m", help = "beet command to execute. Defaults to 'beet import -m'. '.' will be appended to the command, and the execution directory will be set as the yt-dlp download directory.")]
    pub beet: PosixCommand,
    #[arg(
        long,
        visible_alias("acoustid-api-key"),
        help = "API key that will be used for fingerprint lookup"
    )]
    pub acoustid_key: String,
    #[arg(
        long,
        default_value = "warning",
        help = "-loglevel to pass to ffmpeg commands"
    )]
    pub ffmpeg_loglevel: String,
    #[arg(
        long,
        default_value = "16",
        alias = "max-request",
        help = "maximum amount of video requests that can be enqueued by request instances (this does not include the request currently being processed). Defaults to 16, will be coerced between 1 and 128 inclusive"
    )]
    pub max_requests: usize,
    #[arg(long, default_value = "never", value_parser = parse_prompt_flag, help = "Valid values: 'never', 'always', 'ask', defaults to never. It controls whether or not to keep the tmp directory where the commands are executed")]
    pub keep_tmp: PromptFlag,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PromptFlag {
    Always,
    Ask,
    Never,
}

fn parse_prompt_flag(prompt: &str) -> Result<PromptFlag, DaemonError> {
    match prompt {
        "always" => Ok(PromptFlag::Always),
        "ask" => Ok(PromptFlag::Ask),
        "never" => Ok(PromptFlag::Never),
        _ => Err(DaemonError::InvalidKeepTmp {
            provided: prompt.to_string(),
        }),
    }
}

fn parse_yt_dlp(raw: &str) -> Result<PosixCommand, DaemonError> {
    PosixCommand::from_raw(raw).ok_or_else(|| DaemonError::YtDlpCommand {
        erroneous_command: raw.to_string(),
    })
}

fn parse_beet(raw: &str) -> Result<PosixCommand, DaemonError> {
    PosixCommand::from_raw(raw).ok_or_else(|| DaemonError::BeetCommand {
        erroneous_command: raw.to_string(),
    })
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum DaemonError {
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

#[derive(Clone)]
pub(crate) struct TtyState {
    pub vreq_sender: tokio::sync::mpsc::Sender<VideoRequest>,
}

pub(crate) async fn run(args: TtyArgs) {
    let mut lock = lock::get_lock()
        .await
        .expect("Failed to create lock to lockfile");
    let mut guard = lock
        .try_write()
        .expect("Failed to acquire lock guard, is another daemon instance already running?");

    lock::write_pid(&mut guard)
        .await
        .expect("Failed to write PID to lockfile!");

    let tcpl = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind TCP listener");
    let port = tcpl.local_addr().unwrap().port();

    lock::write_port(&mut guard, port)
        .await
        .expect("Failed to write TCP listener port to portfile!");

    // using a mpsc queue lets us asynchronously add to the queue but handle the requests one at a time in the terminal
    let (tx, mut rx) = tokio::sync::mpsc::channel(args.max_requests.clamp(1, 128));

    let app = axum::Router::new()
        .route("/video-request", post(post_video_request))
        .with_state(TtyState { vreq_sender: tx });

    println!(
        "{} Listening on 127.0.0.1:{}",
        style("Daemon is running!").green(),
        port
    );

    tokio::spawn(async move {
        let mut acoustid_client = reqwest::Client::builder()
            .connector_layer(
                ServiceBuilder::new()
                    .layer(tower::buffer::BufferLayer::new(16))
                    .layer(tower::timeout::TimeoutLayer::new(Duration::from_secs(2)))
                    .layer(tower::limit::RateLimitLayer::new(3, Duration::from_secs(1))),
            )
            .https_only(true)
            .build()
            .expect("Could not initialize acoust_id reqwest client.");

        while let Some(vreq) = rx.recv().await {
            let result =
                handle_requests::handle_video_request(vreq, &args, &mut acoustid_client).await;

            match result {
                Ok(true) => {}
                Ok(false) => {}
                Err(error) => {
                    eprintln!(
                        "{}\n{error}",
                        style("Failed to handle video request!").for_stderr().red()
                    )
                }
            }
        }
    });

    axum::serve(tcpl, app).await.unwrap();
}

struct ErrorResponse {
    status_code: axum::http::StatusCode,
    msg: String,
}

impl ErrorResponse {
    pub fn new(status_code: axum::http::StatusCode, msg: String) -> Self {
        Self { status_code, msg }
    }
}

impl axum::response::IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (self.status_code, self.msg).into_response()
    }
}

async fn post_video_request(
    axum::extract::State(state): axum::extract::State<TtyState>,
    axum::Form(vreq): axum::Form<VideoRequest>,
) -> Result<(), ErrorResponse> {
    match state.vreq_sender.try_send(vreq) {
        Ok(_) => Ok(()),
        Err(error) => match error {
            TrySendError::Full(_) => Err(ErrorResponse::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                String::from("Cannot enqueue: Video request queue capacity exceeded!"),
            )),
            // shouldn't happen
            TrySendError::Closed(_) => Err(ErrorResponse::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                String::from("Cannot enqueue: Video request queue closed!"),
            )),
        },
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WithExitStatus<T> {
    pub exit_status: std::process::ExitStatus,
    pub data: T,
}

pub(crate) async fn wait_for_cmd(
    mut child: tokio::process::Child,
) -> Result<WithExitStatus<()>, std::io::Error> {
    child.wait().await.map(|status| status.with_unit())
}

pub(crate) async fn wait_for_cmd_output(
    child: tokio::process::Child,
) -> Result<WithExitStatus<std::process::Output>, std::io::Error> {
    child
        .wait_with_output()
        .await
        .map(|output| output.status.with(output))
}

pub(crate) async fn wrap_command_print_context<T, Ex, FT, FTErr>(
    full_command: &[impl AsRef<str>],
    work_dir: &Path,
    user_settings: impl FnOnce(tokio::process::Command) -> tokio::process::Command,
    extract: Ex,
) -> Result<WithExitStatus<T>, anyhow::Error>
where
    Ex: FnOnce(tokio::process::Child) -> FT,
    FT: Future<Output = Result<WithExitStatus<T>, FTErr>>,
    FTErr: std::error::Error + Send + Sync + 'static,
{
    let full_command = full_command.iter().map(AsRef::as_ref).collect::<Vec<_>>();

    static SEPARATOR: once_cell::sync::OnceCell<String> = once_cell::sync::OnceCell::new();

    let separator: &str = SEPARATOR.get_or_init(|| {
        let width = console::Term::stdout().size().1 as usize;
        let mut sep = String::with_capacity(width);
        for _ in 0..width {
            sep.push('=');
        }
        sep
    });

    println!();
    println!("{}", style(separator).cyan());
    println!("Entering command context.");
    println!("Executing: {}", full_command.join(" "));
    println!("{}", style(separator).cyan());
    println!();

    let mut command = tokio::process::Command::new(full_command[0]);
    command.args(&full_command[1..]);
    command.current_dir(work_dir);
    let mut command = user_settings(command);
    let child = command.spawn()?;
    let result = extract(child).await?;

    println!();
    println!("{}", style(separator).yellow());
    println!("Returned to daemon context.");
    if result.exit_status.success() {
        println!(
            "{}",
            style(format!(
                "Command returned exit code {}.",
                &result.exit_status.code().unwrap()
            ))
            .green()
        );
    } else if let Some(err_code) = result.exit_status.code() {
        println!(
            "{}",
            style(format!("Command returned exit code {}.", err_code)).red()
        );
    } else {
        println!("{}", style("Command was terminated by signal.").red());
    }
    println!("{}", style(separator).yellow());
    println!();

    Ok(result)
}
