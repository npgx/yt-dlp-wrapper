use crate::user::WhatToDo;
use crate::{cli, double_loop_what_to_do, double_loop_what_to_do_opt, fingerprinting, handle_ctrlc, process};
use console::style;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub(crate) struct VideoRequest {
    pub(crate) youtube_id: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum VideoRequestUrlParseError {
    #[error("Unknown url kind")]
    UnknownUrlKind(Url),
}

impl VideoRequest {
    pub(crate) fn from_yt_url(youtube_url: &str) -> Result<Self, anyhow::Error> {
        let youtube_url: Url = youtube_url.parse()?;
        let host_str = youtube_url.host_str().unwrap_or_default();

        let segments = youtube_url
            .path_segments()
            .map(|segments| segments.collect::<Vec<_>>())
            .unwrap_or_default();

        let id: String = if host_str.ends_with("youtube.com") || host_str.ends_with("youtube-nocookie.com") {
            static SEGMENTS_2: [&str; 5] = ["watch", "v", "embed", "e", "shorts"];

            if segments.len() == 1 && segments[0] == "watch" {
                // handle ...youtube.com/watch?v=XXXXXXXXXXX&foo=bar
                let mut pairs = youtube_url.query_pairs();
                let (_, v) = pairs.find(|(k, _)| k == "v").unwrap();
                v.to_string()
            } else if segments.len() == 2 && SEGMENTS_2.contains(&segments[0]) {
                // handle ...youtube.com/(watch|v)/XXXXXXXXXXX?foo=bar
                segments[1].to_string()
            } else {
                return Err(VideoRequestUrlParseError::UnknownUrlKind(youtube_url).into());
            }
        } else if host_str.ends_with("youtu.be") {
            if segments.len() == 1 {
                // handle ...youtu.be/XXXXXXXXXXX?foo=bar
                segments[0].to_string()
            } else {
                return Err(VideoRequestUrlParseError::UnknownUrlKind(youtube_url).into());
            }
        } else {
            // I got lazy: https://gist.github.com/rodrigoborgesdeoliveira/987683cfbfcc8d800192da1e73adc486
            return Err(VideoRequestUrlParseError::UnknownUrlKind(youtube_url).into());
        };

        Ok(Self { youtube_id: id })
    }
}

pub(crate) async fn spawn_video_request_handler(
    mut vreq_receive: tokio::sync::mpsc::Receiver<VideoRequest>,
    args: Arc<cli::TtyArgs>,
) {
    tokio::spawn(async move {
        let mut acoustid_client = reqwest::Client::builder()
            .connector_layer(
                tower::ServiceBuilder::new()
                    .layer(tower::buffer::BufferLayer::new(16))
                    .layer(tower::timeout::TimeoutLayer::new(Duration::from_secs(2)))
                    .layer(tower::limit::RateLimitLayer::new(3, Duration::from_secs(1))),
            )
            .https_only(true)
            .build()
            .expect("Could not initialize acoust_id reqwest client.");

        while let Some(vreq) = vreq_receive.recv().await {
            let result = process_video_request(vreq, &args, &mut acoustid_client).await;

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
}

pub(crate) type RanToCompletion = bool;
pub(crate) async fn process_video_request(
    request: VideoRequest,
    args: &cli::TtyArgs,
    acoustid_client: &mut reqwest::Client,
) -> Result<RanToCompletion, anyhow::Error> {
    'request: loop {
        println!("Processing request for {}", &request.youtube_id);

        let work_dir = tempfile::tempdir()?;
        let work_dir_path = work_dir.path();

        let mut ytdlp_cmd: Vec<&str> = Vec::with_capacity(args.yt_dlp_args.components.len() + 1);
        ytdlp_cmd.push(args.yt_dlp_display.get().unwrap());
        for component in &args.yt_dlp_args.components {
            ytdlp_cmd.push(component); // coerces &String into &str
        }
        ytdlp_cmd.push("--");
        ytdlp_cmd.push(&request.youtube_id);

        'last_command: loop {
            let yt_dlp_command_execution =
                process::handle_child_command_execution(&ytdlp_cmd, work_dir_path, |cmd| cmd, process::wait_for_child)
                    .await?
                    .into_success_or_ask_wtd(|status, _unit| {
                        let message = format!("yt-dlp returned a non-zero exit code: {}", status);

                        (style(message).red(), WhatToDo::all())
                    })
                    .await?;

            match yt_dlp_command_execution {
                Ok(_unit) => {
                    break 'last_command;
                }
                Err(what_to_do) => {
                    double_loop_what_to_do!(what_to_do, 'request, 'last_command, Ok(false));
                }
            }
        }

        'fingerprinting: loop {
            handle_ctrlc!(restart: { continue 'request }, abort: { break 'request Ok(false) });
            let what_to_do =
                fingerprinting::file::handle_fingerprinting_process_for_directory(work_dir_path, acoustid_client, args)
                    .await?;

            double_loop_what_to_do_opt!(what_to_do, 'request, 'fingerprinting, Ok(false), none: { break 'fingerprinting });
        }

        let mut beet_cmd: Vec<&str> = Vec::with_capacity(args.beet_args.components.len() + 1);
        beet_cmd.push(args.beet_display.get().unwrap());
        for component in &args.beet_args.components {
            beet_cmd.push(component); // coerces &String into &str
        }
        beet_cmd.push(".");

        'last_command: loop {
            let beet_command_execution =
                process::handle_child_command_execution(&beet_cmd, work_dir_path, |cmd| cmd, process::wait_for_child)
                    .await?
                    .into_success_or_ask_wtd(|status, _unit| {
                        let message = format!("beet returned a non-zero exit code: {}", status);
                        (style(message).red(), WhatToDo::all())
                    })
                    .await?;

            match beet_command_execution {
                Ok(_unit) => {
                    break 'last_command;
                }
                Err(what_to_do) => {
                    double_loop_what_to_do!(what_to_do, 'request, 'last_command, Ok(false));
                }
            }
        }

        let do_keep_tempdir = match args.keep_tmp {
            cli::PromptFlag::Always => true,
            cli::PromptFlag::Never => false,
            cli::PromptFlag::Ask => {
                let work_dir_path_display = work_dir_path.display().to_string();
                tokio::task::spawn_blocking(move || {
                    dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                        .with_prompt(format!(
                            "Would you like to {} the temp directory '{}'?",
                            style("keep").yellow(),
                            work_dir_path_display
                        ))
                        .default(false)
                        .show_default(true)
                        .wait_for_newline(true)
                        .interact()
                })
                .await??
            }
        };

        handle_ctrlc!(restart: { continue 'request }, abort: { break 'request Ok(false) });

        if do_keep_tempdir {
            let work_dir = work_dir.into_path();
            println!("Persisted directory '{}'", work_dir.display());
        }

        break 'request Ok(true);
    }
}
