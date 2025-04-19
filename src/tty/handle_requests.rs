use crate::tools::acoustid::response::LookupResultsEntry;
use crate::tty::WithExitStatus;
use crate::{tools, tty};
use musicbrainz_rs::entity::artist_credit::ArtistCredit;
use musicbrainz_rs::Fetch;
use std::path::Path;
use std::process::ExitStatus;

pub(crate) type DidRun = bool;
pub(crate) async fn handle_video_request(
    request: tty::VideoRequest,
    args: &tty::TtyArgs,
    acoustid_client: &mut reqwest::Client,
) -> Result<DidRun, anyhow::Error> {
    'request: loop {
        println!("Processing request for {}", &request.youtube_id);

        let work_dir = tempfile::tempdir()?;

        let mut ytdlp_cmd: Vec<&str> = Vec::with_capacity(args.yt_dlp.components.len());
        for component in &args.yt_dlp.components {
            ytdlp_cmd.push(component);
        }
        ytdlp_cmd.push("--");
        ytdlp_cmd.push(&request.youtube_id);

        'last_command: loop {
            let yt_dlp_exit_status = tty::wrap_command_print_context(
                &ytdlp_cmd,
                work_dir.path(),
                |cmd| cmd,
                tty::wait_for_cmd,
            )
            .await?;

            if !yt_dlp_exit_status.exit_status.success() {
                match ask_action_on_command_error(true).await? {
                    WhatToDo::RetryLastCommand => continue 'last_command,
                    WhatToDo::RestartRequest => continue 'request,
                    WhatToDo::Continue => break 'last_command,
                    WhatToDo::AbortRequest => break 'request Ok(false),
                }
            }

            break 'last_command;
        }

        'last_command: loop {
            if let Some(todo) =
                handle_workdir_fingerprinting(work_dir.path(), acoustid_client, args).await?
            {
                match todo {
                    WhatToDo::RetryLastCommand => continue 'last_command,
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
            let beet_exit_status = tty::wrap_command_print_context(
                &beet_cmd,
                work_dir.path(),
                |cmd| cmd,
                tty::wait_for_cmd,
            )
            .await?;

            if !beet_exit_status.exit_status.success() {
                match ask_action_on_command_error(true).await? {
                    WhatToDo::RetryLastCommand => continue 'last_command,
                    WhatToDo::RestartRequest => continue 'request,
                    WhatToDo::Continue => break 'last_command,
                    WhatToDo::AbortRequest => break 'request Ok(false),
                }
            }

            break 'last_command;
        }

        let do_keep_tempdir = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Would you like to keep the temp directory '{}'?",
                work_dir.path().display()
            ))
            .default(false)
            .show_default(true)
            .wait_for_newline(true)
            .interact()?;

        if do_keep_tempdir {
            let work_dir = work_dir.into_path();
            println!("Persisted directory '{}'", work_dir.display());
        }

        break 'request Ok(true);
    }
}

pub(crate) enum WhatToDo {
    RetryLastCommand,
    RestartRequest,
    Continue,
    AbortRequest,
}

impl WhatToDo {
    pub(crate) const fn options_display() -> &'static [&'static str] {
        const OPTIONS: [&str; 4] = [
            "Retry last command",
            "Restart request",
            "Continue to the next command",
            "Abort the request",
        ];
        &OPTIONS
    }
    pub(crate) const fn options_display_no_continue() -> &'static [&'static str] {
        const OPTIONS: [&str; 3] = ["Retry last command", "Restart request", "Abort the request"];
        &OPTIONS
    }

    pub(crate) fn from_ordinal(ordinal: usize) -> Option<WhatToDo> {
        match ordinal {
            0 => Some(WhatToDo::RetryLastCommand),
            1 => Some(WhatToDo::RestartRequest),
            2 => Some(WhatToDo::Continue),
            3 => Some(WhatToDo::AbortRequest),
            _ => None,
        }
    }

    pub(crate) fn from_ordinal_no_continue(ordinal: usize) -> Option<WhatToDo> {
        match ordinal {
            0 => Some(WhatToDo::RetryLastCommand),
            1 => Some(WhatToDo::RestartRequest),
            2 => Some(WhatToDo::AbortRequest),
            _ => None,
        }
    }
}

pub(crate) async fn ask_action_on_command_error(
    allow_continue: bool,
) -> Result<WhatToDo, anyhow::Error> {
    let todo = tokio::task::spawn_blocking(move || {
        if allow_continue {
            dialoguer::Select::new()
                .with_prompt(
                    "The last executed command returned a non-zero exit code, what would you like to do?",
                )
                .default(0)
                .items(WhatToDo::options_display())
                .interact()
                .map(|ordinal| WhatToDo::from_ordinal(ordinal).unwrap())
        } else {
            dialoguer::Select::new()
                .with_prompt(
                    "The last executed command returned a non-zero exit code, what would you like to do?",
                )
                .default(0)
                .items(WhatToDo::options_display_no_continue())
                .interact()
                .map(|ordinal| WhatToDo::from_ordinal_no_continue(ordinal).unwrap())
        }
    }).await??;

    Ok(todo)
}

fn get_fingerprintable_directory_contents(path: &Path) -> Vec<String> {
    let contents = match std::fs::read_dir(path) {
        Ok(contents) => contents,
        Err(_) => return vec![],
    };

    contents
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

async fn handle_workdir_fingerprinting(
    work_dir: &Path,
    acoustid_client: &mut reqwest::Client,
    args: &tty::TtyArgs,
) -> Result<Option<WhatToDo>, anyhow::Error> {
    let fingerprintable = get_fingerprintable_directory_contents(work_dir);

    let mut defaults = vec![true; fingerprintable.len() + 1];
    defaults[0] = false;

    let mut selections = dialoguer::MultiSelect::new()
        .with_prompt(
            "Select files to fingerprint, if <none> is selected, all other selections will be ignored",
        )
        .item("<none>")
        .items(&fingerprintable)
        .defaults(&defaults)
        .max_length(16)
        .interact()?;

    // is <none> selected
    if selections.contains(&0) {
        println!("Fingerprinting none!");
        selections.clear();
    }

    let mut to_fingerprint = vec![];
    for selection in selections {
        to_fingerprint.push(fingerprintable[selection - 1].clone());
    }

    for filename in to_fingerprint {
        let filepath = work_dir.join(&filename);
        if let Some(todo) = handle_file_fingerprinting(&filepath, acoustid_client, args).await? {
            return Ok(Some(todo));
        }
    }

    Ok(None)
}

async fn handle_file_fingerprinting(
    filepath: &Path,
    acoustid_client: &mut reqwest::Client,
    args: &tty::TtyArgs,
) -> Result<Option<WhatToDo>, anyhow::Error> {
    let (fingerprint, track_duration) = match tools::fingerprint_file(filepath).await? {
        Ok(data) => data,
        Err(todo) => return Ok(Some(todo)),
    };

    let fingerprint_lookup = tools::acoustid::lookup_fingerprint(
        acoustid_client,
        fingerprint,
        track_duration.floor() as u64,
        &args.acoustid_key,
    )
    .await?;

    if fingerprint_lookup.status != "ok" {
        return Err(anyhow::anyhow!(
            "AcoustID fingerprint lookup failed with status (inside http response JSON body) {}",
            fingerprint_lookup.status
        ));
    }

    let results = fingerprint_lookup.results.unwrap_or_else(Vec::new);

    let (results_with_recordings, results_others): (Vec<_>, Vec<_>) =
        results.into_iter().partition(|entry| {
            entry
                .recordings
                .as_ref()
                .is_some_and(|recs| !recs.is_empty())
        });

    if !results_others.is_empty() {
        println!(
            "Ignoring {} matches that do not have any associated recordings!",
            results_others.len()
        );
    }

    if results_with_recordings.is_empty() {
        println!("No AcoustID matches with associated recordings!");
        return Ok(None);
    }

    let results_display: Vec<String> = results_with_recordings
        .iter()
        .map(|entry| {
            format!(
                "Score: {}, AcoustID: {}, Recordings: {}",
                &entry.score,
                &entry.id,
                entry.recordings.as_ref().unwrap().len()
            )
        })
        .collect();

    let runtime = tokio::runtime::Handle::current();
    let selection = tokio::task::spawn_blocking(move || {
        get_recording_from_selection_tree(&results_with_recordings, &results_display, runtime)
    })
    .await?;

    if let Some(selection) = selection {
        let movedir = tempfile::tempdir()?;

        let moved_filepath = movedir.path().join(filepath.file_name().unwrap());
        println!(
            "Moving '{}' to '{}'",
            &filepath.display(),
            &moved_filepath.display()
        );
        std::fs::rename(filepath, &moved_filepath)?;

        let ffmpeg_cmd = [
            "ffmpeg",
            "-loglevel",
            &args.ffmpeg_loglevel,
            "-i",
            &moved_filepath.display().to_string(),
            "-metadata",
            &format!("MusicBrainz Track Id={}", selection.id),
            "-codec",
            "copy",
            &filepath.display().to_string(),
        ];

        'last_command: loop {
            let ffmpeg_exit_status = tty::wrap_command_print_context(
                &ffmpeg_cmd,
                movedir.path(),
                |cmd| cmd,
                tty::wait_for_cmd,
            )
            .await?;

            if !ffmpeg_exit_status.exit_status.success() {
                match ask_action_on_command_error(true).await? {
                    WhatToDo::RetryLastCommand => continue 'last_command,
                    WhatToDo::RestartRequest => return Ok(Some(WhatToDo::RestartRequest)),
                    WhatToDo::Continue => break 'last_command,
                    WhatToDo::AbortRequest => return Ok(Some(WhatToDo::AbortRequest)),
                }
            }

            break 'last_command;
        }

        println!(
            "Copied '{}' to '{}' with MusicBrainz track id metadata",
            moved_filepath.display(),
            filepath.display()
        );

        Ok(None)
    } else {
        Ok(None)
    }
}

fn get_recording_from_selection_tree<'l>(
    results: &'l [LookupResultsEntry],
    results_display: &[String],
    runtime: tokio::runtime::Handle,
) -> Option<tools::acoustid::response::RecordingEntry> {
    let ask_top_level = || {
        dialoguer::Select::new()
            .item("<none>")
            .items(results_display)
            .default(0)
            .max_length(16)
            .interact()
            .ok()
            .filter(|index| *index > 0)
            .map(|index| &results[index - 1])
    };

    let ask_results = |entry: &'l LookupResultsEntry| {
        let recordings = entry.recordings.as_ref().unwrap();
        let recordings_display: Vec<String> = recordings.iter().map(|entry| {
            let recording = runtime.block_on(
                musicbrainz_rs::entity::recording::Recording::fetch()
                    .id(&entry.id)
                    .with_artists()
                    .execute(),
            );

            fn artists_to_string(data: Vec<ArtistCredit>) -> String {
                let mut res = String::new();
                for artist in data {
                    res.push_str(&artist.name);
                    res.push(' ');
                    if let Some(joinphrase) = &artist.joinphrase {
                        res.push_str(joinphrase);
                        res.push(' ');
                    }
                }
                res
            }

            match recording {
                Ok(data) => {
                    format!(
                        "https://musicbrainz.org/recording/{}; Title: {}, Disambiguation: {}, Artists: {}",
                        entry.id,
                        data.title,
                        data.disambiguation.unwrap_or_default(),
                        artists_to_string(data.artist_credit.unwrap_or_else(Vec::new))
                    )
                }
                Err(_) => {
                    format!(
                        "https://musicbrainz.org/recording/{}; API request to MusicBrainz failed!",
                        entry.id
                    )
                }
            }
        }).collect();

        dialoguer::Select::new()
            .with_prompt(format!("Select <back> to go back to the previous selection. Currently exploring AcoustID: {}", entry.id))
            .item("<back>")
            .items(&recordings_display)
            .default(0)
            .max_length(16)
            .interact()
            .ok()
            .filter(|index| *index > 0)
            .map(|index| &recordings[index - 1])
    };

    println!(
        "Explore the various associated recordings: Select <none> if none is correct, otherwise, when an option is selected, a nested selection for the correct MusicBrainz recording will appear"
    );

    'outer: loop {
        match ask_top_level() {
            Some(entry) => 'inner: loop {
                match ask_results(entry) {
                    None => continue 'outer,
                    Some(record) => {
                        let confirm = dialoguer::Confirm::new()
                            .with_prompt(format!(
                                "Confirm recording: https://musicbrainz.org/recording/{}",
                                &record.id
                            ))
                            .default(true)
                            .show_default(true)
                            .wait_for_newline(true)
                            .interact()
                            .is_ok_and(|r| r);

                        if confirm {
                            return Some(record.clone());
                        } else {
                            continue 'inner;
                        }
                    }
                }
            },
            None => return None,
        }
    }
}

pub(crate) trait ExitStatusExt {
    fn with<T>(self, data: T) -> WithExitStatus<T>;
    fn with_unit(self) -> WithExitStatus<()>;
}

impl ExitStatusExt for ExitStatus {
    fn with<T>(self, data: T) -> WithExitStatus<T> {
        WithExitStatus {
            exit_status: self,
            data,
        }
    }

    fn with_unit(self) -> WithExitStatus<()> {
        WithExitStatus {
            exit_status: self,
            data: (),
        }
    }
}
