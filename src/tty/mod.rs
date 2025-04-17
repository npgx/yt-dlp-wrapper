use crate::lock;
use crate::video::VideoRequest;
use axum::routing::post;
use dialoguer::Confirm;
use std::error::Error;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::process::ExitStatus;

#[derive(clap::Args, Debug, Clone)]
pub(crate) struct TtyArgs {
    #[arg(long, visible_alias("yt-dlp-command"), value_parser = parse_yt_dlp, help = "yt-dlp command to execute. NOTE: '--' will automatically be appended to this. NOTE: each command chain will execute in a different temporary directory.")]
    pub yt_dlp: PosixCommand,
    #[arg(long, visible_alias("beet-command"), value_parser = parse_beet, default_value = "beet import -m", help = "beet command to execute. Defaults to 'beet import -m'. '.' will be appended to the command, and the execution directory will be set as the yt-dlp download directory.")]
    pub beet: PosixCommand,
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
        shlex::split(raw).map(|split| Self::new(split))
    }
}

impl Display for PosixCommand {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/*pub(crate) struct InOut {
    from: Option<SocketAddr>,
    stdin_join: Option<tokio::task::JoinHandle<()>>,
    pub stdin: sluice::pipe::PipeReader,
    stdout_join: Option<tokio::task::JoinHandle<()>>,
    pub stdout: sluice::pipe::PipeWriter,
}

impl InOut {
    pub(crate) async fn std() -> Self {
        let (myin, mut myin_writer) = sluice::pipe::pipe();
        let (mut myout_reader, myout) = sluice::pipe::pipe();

        let stdin_join = tokio::spawn(async move {
            let buf = Arc::new(tokio::sync::Mutex::new(vec![0; BUFFER_SIZE]));
            loop {
                let lambda_buf = buf.clone();
                let read_count = tokio::task::spawn_blocking(move || {
                    std::io::stdin()
                        .read(&mut *lambda_buf.blocking_lock())
                        .expect("Error reading from stdin")
                })
                .await
                .expect("Failed to join stdin read blocking task");

                if read_count == 0 {
                    break;
                }

                if let Err(_) = myin_writer.write_all(&buf.lock().await[..read_count]).await {
                    break;
                }
            }
        });

        let stdout_join = tokio::spawn(async move {
            let buf = Arc::new(tokio::sync::Mutex::new(vec![0; BUFFER_SIZE]));
            while let Ok(read_count) = myout_reader.read(&mut *buf.lock().await).await {
                if read_count == 0 {
                    break;
                }

                let lambda_buf = buf.clone();
                tokio::task::spawn_blocking(move || {
                    std::io::stdout()
                        .write_all(&lambda_buf.blocking_lock()[..read_count])
                        .expect("Error writing to stdout");
                })
                .await
                .expect("Failed to join stdout write task");
            }
        });

        InOut {
            from: None,
            stdin_join: Some(stdin_join),
            stdin: myin,
            stdout_join: Some(stdout_join),
            stdout: myout,
        }
    }
}

impl Drop for InOut {
    fn drop(&mut self) {
        if let Some(handle) = self.stdin_join.take() {
            handle.abort();
        }
        if let Some(handle) = self.stdout_join.take() {
            handle.abort();
        }
    }
}*/

/*pub(crate) static INOUT: once_cell::sync::Lazy<tokio::sync::Mutex<Option<InOut>>> =
once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(None));*/

#[derive(Clone)]
pub(crate) struct TtyState {
    pub vreq_sender: tokio::sync::mpsc::Sender<VideoRequest>,
}

pub(crate) async fn run(args: TtyArgs) {
    /* *INOUT.lock().await = Some(InOut::std().await);*/

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
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);

    let app = axum::Router::new()
        .route("/video-request", post(post_video_request))
        .with_state(TtyState { vreq_sender: tx });

    println!("Daemon is running! Listening on 127.0.0.1:{}", port);

    tokio::spawn(async move {
        while let Some(vreq) = rx.recv().await {
            handle_video_request(vreq, &args)
                .await
                .expect("Failed to handle video request!");
        }
    });

    axum::serve(tcpl, app).await.unwrap();
}

async fn post_video_request(
    axum::extract::State(state): axum::extract::State<TtyState>,
    axum::Form(vreq): axum::Form<VideoRequest>,
) -> Result<(), String> {
    state
        .vreq_sender
        .send(vreq)
        .await
        .map_err(|err| format!("{err:?}"))?;
    Ok(())
}

type DidRun = bool;
async fn handle_video_request(
    request: VideoRequest,
    args: &TtyArgs,
) -> Result<DidRun, Box<dyn Error + Send + Sync>> {
    println!("Received request for {}", request.youtube_id);

    // ONLY pass this directory to pretty_ask_execute behind a reference, otherwise, the command execution will fail
    // because work_dir will get dropped in the coercion from TempDir to Path (impl AsRef<Path>), which will delete the directory
    // and print the error: Os { code: 2, kind: NotFound, message: "No such file or directory" }
    let work_dir = tempfile::tempdir()?;

    let mut ytdlp_cmd = args.yt_dlp.components.clone();
    ytdlp_cmd.push(String::from("--"));
    ytdlp_cmd.push(request.youtube_id);
    match pretty_ask_execute(ytdlp_cmd, &work_dir).await? {
        PrettyAskCommandStatus::CorrectlyExecuted => {}
        PrettyAskCommandStatus::ExecutedWithNonZeroExit(code) => {}
        PrettyAskCommandStatus::UserCancelled => {}
    }

    let mut beet_cmd = args.beet.components.clone();
    beet_cmd.push(String::from("."));
    match pretty_ask_execute(beet_cmd, &work_dir).await? {
        PrettyAskCommandStatus::CorrectlyExecuted => {}
        PrettyAskCommandStatus::ExecutedWithNonZeroExit(code) => {}
        PrettyAskCommandStatus::UserCancelled => {}
    }

    let do_persist_tempdir = Confirm::new()
        .with_prompt(format!(
            "Would you like to persist the temp directory '{}'?",
            work_dir.path().display()
        ))
        .default(false)
        .show_default(true)
        .report(false)
        .interact()
        .map_err(|err| Box::new(err))?;

    if do_persist_tempdir {
        let work_dir = work_dir.into_path();
        println!("Persisted directory '{}'", work_dir.display());
    }

    Ok(true)
}

fn print_enter_command_context() {
    println!();
    println!("================================");
    println!("Entering command context.");
    println!("================================");
    println!();
}

fn print_return_daemon_context(return_code: Option<i32>) {
    println!();
    println!("================================");
    println!("Returned to daemon context.");
    match return_code {
        Some(code) => println!("Command returned exit code {}.", code),
        None => println!("Command was terminated by signal."),
    }
    println!("================================");
    println!();
}

enum PrettyAskCommandStatus {
    CorrectlyExecuted,
    ExecutedWithNonZeroExit(ExitStatus),
    UserCancelled,
}

async fn pretty_ask_execute(
    full_command: Vec<String>,
    work_dir: impl AsRef<Path>,
) -> Result<PrettyAskCommandStatus, Box<dyn Error + Send + Sync>> {
    println!(
        "Would you like to execute the following command in '{}'\n{}",
        work_dir.as_ref().display(),
        full_command.join(" ")
    );
    let confirmation = Confirm::new()
        .default(true)
        .show_default(true)
        .report(false)
        .interact()
        .map_err(|err| Box::new(err))?;

    if !confirmation {
        println!("Command not executed!");
        return Ok(PrettyAskCommandStatus::UserCancelled);
    }

    print_enter_command_context();
    let exit_status = tokio::process::Command::new(&full_command[0])
        .args(&full_command[1..])
        .current_dir(work_dir)
        .spawn()?
        .wait()
        .await?;
    print_return_daemon_context(exit_status.code());

    if exit_status.success() {
        Ok(PrettyAskCommandStatus::CorrectlyExecuted)
    } else {
        Ok(PrettyAskCommandStatus::ExecutedWithNonZeroExit(exit_status))
    }
}
