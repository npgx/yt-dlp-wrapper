use crate::video::VideoRequest;
use crate::{lock, tools};
use axum::routing::post;
use dialoguer::theme::ColorfulTheme;
use dialoguer::Confirm;
use itertools::Itertools;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;
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
        shlex::split(raw).map(Self::new)
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

    ffmpeg_next::init().expect("Could not initialize ffmpeg library");

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
        let mut acoustid_client = reqwest::Client::builder()
            .connector_layer(
                ServiceBuilder::new()
                    .layer(tower::buffer::BufferLayer::new(16))
                    .layer(tower::timeout::TimeoutLayer::new(Duration::from_secs(2)))
                    .layer(tower::limit::RateLimitLayer::new(3, Duration::from_secs(1))),
            )
            .build()
            .expect("Could not initialize acoust_id reqwest client.");

        while let Some(vreq) = rx.recv().await {
            handle_video_request(vreq, &args, &mut acoustid_client)
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
    acoustid_client: &mut reqwest::Client,
) -> Result<DidRun, anyhow::Error> {
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
        PrettyAskCommandStatus::ExecutedWithNonZeroExit(_code) => {}
        PrettyAskCommandStatus::UserCancelled => {}
    }

    {
        let mut contents = std::fs::read_dir(work_dir.path())?
            .filter_map(|entry| {
                entry
                    .ok()
                    .and_then(|entry| entry.file_name().into_string().ok())
            })
            .collect::<Vec<_>>();
        contents.insert(0, String::from("<none>"));

        let mut defaults = vec![true; contents.len()];
        defaults[0] = false;

        let mut selections = dialoguer::MultiSelect::with_theme(&ColorfulTheme::default()).with_prompt(
            "Select files to fingerprint, if <none> is selected, all other selections will be ignored",
        )
            .items(&contents)
            .defaults(&defaults)
            .max_length(8)
            .interact()?;

        if selections.contains(&0) {
            selections.clear();
        }

        let mut to_fingerprint = vec![];
        for selection in selections {
            to_fingerprint.push(contents[selection].clone());
        }

        for filename in to_fingerprint {
            let filepath = work_dir.path().join(&filename);
            let (fingerprint, track_duration) = tools::fingerprint_file(&filepath).await?;
            let fingerprint_lookup = tools::acoustid::lookup_fingerprint(
                acoustid_client,
                fingerprint,
                track_duration.floor() as u64,
                &args.acoustid_key,
            )
            .await?;

            let movedir = tempfile::tempdir()?;

            match fingerprint_lookup.results {
                Some(mut results) if fingerprint_lookup.status == "ok" && !results.is_empty() => {
                    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

                    let results_display: Vec<String> = results
                        .iter()
                        .filter_map(|entry| {
                            let empty = vec![];
                            let recordings = entry.recordings.as_ref().unwrap_or(&empty);
                            if recordings.is_empty() {
                                None
                            } else {
                                let recordings_display = recordings
                                    .iter()
                                    .map(|entry| {
                                        format!("https://musicbrainz.org/recording/{}", entry.id)
                                    })
                                    .join(", ");

                                Some(format!(
                                    "Score: {}, AcoustID: {}, Recordings: {}",
                                    &entry.score, &entry.id, recordings_display
                                ))
                            }
                        })
                        .collect();

                    if results_display.is_empty() {
                        eprintln!("No associated AcoustID WITH RECORDINGS found!");
                        continue;
                    }

                    let selection = dialoguer::Select::new()
                        .with_prompt("Found matching AcoustID Tracks! Select the correct one, or <none> if none is correct (a secondary selection will appear when there are multiple associated recordings)")
                        .item("<none>")
                        .items(&results_display)
                        .default(0)
                        .max_length(16)
                        .interact()?;
                    if selection == 0 {
                        continue;
                    }

                    let selection = match &results[selection - 1].recordings {
                        Some(recordings) if !recordings.is_empty() => {
                            if recordings.len() == 1 {
                                println!(
                                    "Automatically choosing recording https://musicbrainz.org/recording/{}",
                                    &recordings[0].id
                                );
                                recordings[0].clone()
                            } else {
                                let recordings_display: Vec<String> = recordings
                                    .iter()
                                    .map(|entry| {
                                        format!("https://musicbrainz.org/recording/{}", entry.id)
                                    })
                                    .collect();
                                let recording_selection = dialoguer::Select::new()
                                    .with_prompt("Choose correct recording")
                                    .items(&recordings_display)
                                    .default(0)
                                    .max_length(16)
                                    .interact()?;

                                recordings[recording_selection].clone()
                            }
                        }
                        _ => {
                            panic!("Internal error!")
                        }
                    };

                    let moved_filepath = movedir.path().join(&filename);
                    println!(
                        "Moving {} to {}",
                        &filepath.display(),
                        &moved_filepath.display()
                    );
                    std::fs::rename(&filepath, &moved_filepath)?;

                    let cmd = [
                        "ffmpeg",
                        "-i",
                        &moved_filepath.display().to_string(),
                        "-metadata",
                        &format!("MusicBrainz Track Id={}", selection.id),
                        "-codec",
                        "copy",
                        &filepath.display().to_string(),
                    ];

                    print_enter_command_context(cmd.join(" "));
                    let output = tokio::process::Command::new(cmd[0])
                        .args(&cmd[1..])
                        .current_dir(moved_filepath.parent().unwrap())
                        .spawn()?
                        .wait()
                        .await?;
                    print_return_daemon_context(output.code());
                }
                _ => {}
            }
        }
    }

    let mut beet_cmd = args.beet.components.clone();
    beet_cmd.push(String::from("."));
    match pretty_ask_execute(beet_cmd, &work_dir).await? {
        PrettyAskCommandStatus::CorrectlyExecuted => {}
        PrettyAskCommandStatus::ExecutedWithNonZeroExit(_code) => {}
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
        .interact()?;

    if do_persist_tempdir {
        let work_dir = work_dir.into_path();
        println!("Persisted directory '{}'", work_dir.display());
    }

    Ok(true)
}

pub(crate) fn print_enter_command_context(full_command: impl AsRef<str>) {
    println!();
    println!("================================");
    println!("Entering command context.");
    println!("Executing: {}", full_command.as_ref());
    println!("================================");
    println!();
}

pub(crate) fn print_return_daemon_context(return_code: Option<i32>) {
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
) -> Result<PrettyAskCommandStatus, anyhow::Error> {
    println!(
        "Would you like to execute the following command in '{}'\n{}",
        work_dir.as_ref().display(),
        full_command.join(" ")
    );
    let confirmation = Confirm::new()
        .default(true)
        .show_default(true)
        .report(false)
        .interact()?;

    if !confirmation {
        println!("Command not executed!");
        return Ok(PrettyAskCommandStatus::UserCancelled);
    }

    print_enter_command_context(full_command.join(" "));
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
