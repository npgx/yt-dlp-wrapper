use crate::cli::TtyArgs;
use crate::user::{ask_what_to_do, WhatToDo};
use crate::{cli, fingerprinting, process};
use console::style;
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
    pub(crate) fn from_yt_url(youtube_url: &str) -> Result<Self, VideoRequestUrlParseError> {
        let youtube_url: Url = youtube_url.parse().unwrap();
        let host_str = youtube_url.host_str().unwrap();

        let id: String = if host_str.ends_with("youtube.com") || host_str.ends_with("youtube-nocookie.com") {
            let segments = youtube_url.path_segments().unwrap().collect::<Vec<_>>();
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
                return Err(VideoRequestUrlParseError::UnknownUrlKind(youtube_url));
            }
        } else if youtube_url.host_str().unwrap().ends_with("youtu.be") {
            let segments = youtube_url.path_segments().unwrap().collect::<Vec<_>>();
            if segments.len() == 1 {
                // handle ...youtu.be/XXXXXXXXXXX?foo=bar
                segments[0].to_string()
            } else {
                return Err(VideoRequestUrlParseError::UnknownUrlKind(youtube_url));
            }
        } else {
            // I got lazy: https://gist.github.com/rodrigoborgesdeoliveira/987683cfbfcc8d800192da1e73adc486
            return Err(VideoRequestUrlParseError::UnknownUrlKind(youtube_url));
        };

        Ok(Self { youtube_id: id })
    }
}

pub(crate) type DidRun = bool;
pub(crate) async fn handle_video_request(
    request: VideoRequest,
    args: &TtyArgs,
    acoustid_client: &mut reqwest::Client,
) -> Result<DidRun, anyhow::Error> {
    'request: loop {
        println!("Processing request for {}", &request.youtube_id);

        let work_dir = tempfile::tempdir()?;
        let work_dir_path = work_dir.path();

        let mut ytdlp_cmd: Vec<&str> = Vec::with_capacity(args.yt_dlp.components.len());
        for component in &args.yt_dlp.components {
            ytdlp_cmd.push(component);
        }
        ytdlp_cmd.push("--");
        ytdlp_cmd.push(&request.youtube_id);

        'last_command: loop {
            let yt_dlp_exit_status =
                process::wrap_command_print_context(&ytdlp_cmd, work_dir_path, |cmd| cmd, process::wait_for_cmd)
                    .await?;

            if !yt_dlp_exit_status.exit_status.success() {
                match ask_what_to_do(
                    style(format!(
                        "yt-dlp returned a non-zero exit code: {}",
                        yt_dlp_exit_status.exit_status
                    ))
                    .red(),
                    WhatToDo::all(),
                )
                .await?
                {
                    WhatToDo::Retry => continue 'last_command,
                    WhatToDo::RestartRequest => continue 'request,
                    WhatToDo::Continue => break 'last_command,
                    WhatToDo::AbortRequest => break 'request Ok(false),
                }
            }

            break 'last_command;
        }

        'last_command: loop {
            if let Some(todo) =
                fingerprinting::file::handle_fingerprinting_process_for_directory(work_dir_path, acoustid_client, args)
                    .await?
            {
                match todo {
                    WhatToDo::Retry => continue 'last_command,
                    WhatToDo::RestartRequest => continue 'request,
                    WhatToDo::Continue => break 'last_command,
                    WhatToDo::AbortRequest => break 'request Ok(false),
                }
            }

            break 'last_command;
        }

        let mut beet_cmd: Vec<&str> = Vec::with_capacity(args.beet.components.len());
        for component in &args.beet.components {
            beet_cmd.push(component);
        }
        beet_cmd.push(".");
        'last_command: loop {
            let beet_exit_status =
                process::wrap_command_print_context(&beet_cmd, work_dir_path, |cmd| cmd, process::wait_for_cmd).await?;

            if !beet_exit_status.exit_status.success() {
                match ask_what_to_do(
                    style(format!(
                        "beet returned a non-zero exit code: {}",
                        beet_exit_status.exit_status
                    ))
                    .red(),
                    WhatToDo::all(),
                )
                .await?
                {
                    WhatToDo::Retry => continue 'last_command,
                    WhatToDo::RestartRequest => continue 'request,
                    WhatToDo::Continue => break 'last_command,
                    WhatToDo::AbortRequest => break 'request Ok(false),
                }
            }

            break 'last_command;
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

        if do_keep_tempdir {
            let work_dir = work_dir.into_path();
            println!("Persisted directory '{}'", work_dir.display());
        }

        break 'request Ok(true);
    }
}
