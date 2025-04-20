use crate::tools::acoustid;
use crate::tools::acoustid::response::{LookupResultsEntry, RecordingEntry};
use crate::tty::{PromptFlag, WithExitStatus};
use crate::{tools, tty};
use musicbrainz_rs::Fetch;
use std::borrow::Cow;
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
        let work_dir_path = work_dir.path();

        let mut ytdlp_cmd: Vec<&str> = Vec::with_capacity(args.yt_dlp.components.len());
        for component in &args.yt_dlp.components {
            ytdlp_cmd.push(component);
        }
        ytdlp_cmd.push("--");
        ytdlp_cmd.push(&request.youtube_id);

        'last_command: loop {
            let yt_dlp_exit_status = tty::wrap_command_print_context(
                &ytdlp_cmd,
                work_dir_path,
                |cmd| cmd,
                tty::wait_for_cmd,
            )
            .await?;

            if !yt_dlp_exit_status.exit_status.success() {
                match ask_action_on_command_error(
                    format!(
                        "yt-dlp returned a non-zero exit code: {}",
                        yt_dlp_exit_status.exit_status
                    ),
                    true,
                )
                .await?
                {
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
                handle_workdir_fingerprinting(work_dir_path, acoustid_client, args).await?
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
                work_dir_path,
                |cmd| cmd,
                tty::wait_for_cmd,
            )
            .await?;

            if !beet_exit_status.exit_status.success() {
                match ask_action_on_command_error(
                    format!(
                        "beet returned a non-zero exit code: {}",
                        beet_exit_status.exit_status
                    ),
                    true,
                )
                .await?
                {
                    WhatToDo::RetryLastCommand => continue 'last_command,
                    WhatToDo::RestartRequest => continue 'request,
                    WhatToDo::Continue => break 'last_command,
                    WhatToDo::AbortRequest => break 'request Ok(false),
                }
            }

            break 'last_command;
        }

        let do_keep_tempdir = match args.keep_tmp {
            PromptFlag::Always => true,
            PromptFlag::Never => false,
            PromptFlag::Ask => {
                let work_dir_path_display = work_dir_path.display().to_string();
                tokio::task::spawn_blocking(move || {
                    dialoguer::Confirm::new()
                        .with_prompt(format!(
                            "Would you like to keep the temp directory '{}'?",
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
    message: String,
    allow_continue: bool,
) -> Result<WhatToDo, anyhow::Error> {
    let todo = tokio::task::spawn_blocking(move || {
        if allow_continue {
            dialoguer::Select::new()
                .with_prompt(format!("{message}\nWhat would you like to do?"))
                .default(0)
                .items(WhatToDo::options_display())
                .interact()
                .map(|ordinal| WhatToDo::from_ordinal(ordinal).unwrap())
        } else {
            dialoguer::Select::new()
                .with_prompt(format!("{message}\nWhat would you like to do?"))
                .default(0)
                .items(WhatToDo::options_display_no_continue())
                .interact()
                .map(|ordinal| WhatToDo::from_ordinal_no_continue(ordinal).unwrap())
        }
    })
    .await??;

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

    let selections_fingerprintable = fingerprintable.clone();
    let mut selections = tokio::task::spawn_blocking(move || {
        dialoguer::MultiSelect::new()
            .with_prompt(
                "Select files to fingerprint, if <none> is selected, all other selections will be ignored",
            )
            .item("<none>")
            .items(&selections_fingerprintable)
            .defaults(&defaults)
            .max_length(16)
            .interact()
    }).await??;

    // is <none> selected
    if selections.contains(&0) {
        println!("Fingerprinting none!");
        selections.clear();
    }

    let mut to_fingerprint = vec![];
    for selection in selections {
        to_fingerprint.push(&fingerprintable[selection - 1]);
    }

    for filename in to_fingerprint {
        let filepath = work_dir.join(filename);
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
    let fpcalc_output = match tools::fingerprint_file(filepath).await? {
        Ok(data) => data,
        Err(todo) => return Ok(Some(todo)),
    };

    let fingerprint_lookup = acoustid::lookup_fingerprint(
        acoustid_client,
        &fpcalc_output.fingerprint,
        fpcalc_output.duration.floor() as u64,
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

    let runtime = tokio::runtime::Handle::current();

    let mut selection = if results_with_recordings.is_empty() {
        println!("No AcoustID matches with associated recordings!");
        None
    } else {
        tokio::task::spawn_blocking(move || {
            get_recording_from_selection_tree(&results_with_recordings, runtime)
        })
        .await?
    };

    if selection.is_none() {
        use once_cell::sync::OnceCell;
        static ACOUSTID_USER_KEY: OnceCell<String> = OnceCell::new();

        let submit = tokio::task::spawn_blocking(move || {
            dialoguer::Confirm::new()
                .with_prompt("Would you like to submit the fingerprint?")
                .default(true)
                .show_default(true)
                .wait_for_newline(true)
                .interact()
        })
        .await??;

        if submit {
            'submit: loop {
                let acoustid_user_key: Cow<str> = if ACOUSTID_USER_KEY.get().is_some() {
                    Cow::Borrowed(ACOUSTID_USER_KEY.get().unwrap())
                } else {
                    // don't set it asap, set it when the request succeeds
                    'api_key: loop {
                        let user_input = tokio::task::spawn_blocking(move || {
                            dialoguer::Input::<String>::new()
                                .with_prompt(
                                    "Insert the AcoustID user API key (https://acoustid.org)",
                                )
                                .allow_empty(false)
                                .report(false)
                                .interact_text()
                        })
                        .await?;
                        match user_input {
                            Ok(value) => break 'api_key Cow::Owned(value),
                            Err(err) => eprintln!("Invalid User Key: {}", err),
                        }
                    }
                };

                let mbid = 'mbid: loop {
                    let tmp = tokio::task::spawn_blocking(move || {
                        dialoguer::Input::<String>::new()
                            .with_prompt("Insert the MusicBrainz RECORDING ID that you would like to bind to the fingerprint")
                            .allow_empty(false)
                            .report(true)
                            .interact_text()
                    }).await?;
                    match tmp {
                        Ok(value) => {
                            let value2 = value.clone();
                            let confirm = tokio::task::spawn_blocking(move || {
                                dialoguer::Confirm::new()
                                    .with_prompt(format!(
                                        "Confirm MusicBrainz RECORDING ID: {}",
                                        value2
                                    ))
                                    .default(true)
                                    .show_default(true)
                                    .wait_for_newline(true)
                                    .interact()
                            })
                            .await??;

                            if confirm {
                                break 'mbid value;
                            } else {
                                continue 'mbid;
                            }
                        }
                        Err(err) => eprintln!("Invalid Record ID: {}", err),
                    }
                };

                let (what_to_do, recording) = acoustid::submit_fingerprint(
                    acoustid_client,
                    &fpcalc_output.fingerprint,
                    fpcalc_output.duration.floor() as u64,
                    &mbid,
                    &acoustid_user_key,
                    args,
                )
                .await?;

                match what_to_do {
                    Some(WhatToDo::RetryLastCommand) => continue 'submit,
                    Some(WhatToDo::RestartRequest) => return Ok(Some(WhatToDo::AbortRequest)),
                    None | Some(WhatToDo::Continue) => {
                        let _ = selection.insert(recording);
                        break 'submit;
                    }
                    Some(WhatToDo::AbortRequest) => return Ok(Some(WhatToDo::AbortRequest)),
                }
            }
        }
    }

    match selection {
        None => Ok(None),
        Some(recording) => {
            let filename = filepath.file_name().unwrap();

            let movedir = tempfile::tempdir()?;
            let moved_filepath = movedir.path().join(filename);

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
                &format!("MusicBrainz Track Id={}", recording.id),
                "-metadata",
                &format!("Title={}", recording.title),
                "-metadata",
                &format!(
                    "Artist={}",
                    artists_to_string(recording.artist_credit.unwrap())
                ),
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
                    match ask_action_on_command_error(
                        format!(
                            "ffmpeg returned a non-zero exit code: {}",
                            ffmpeg_exit_status.exit_status
                        ),
                        true,
                    )
                    .await?
                    {
                        WhatToDo::RetryLastCommand => continue 'last_command,
                        WhatToDo::RestartRequest => return Ok(Some(WhatToDo::RestartRequest)),
                        WhatToDo::Continue => break 'last_command,
                        WhatToDo::AbortRequest => return Ok(Some(WhatToDo::AbortRequest)),
                    }
                }

                break 'last_command;
            }

            println!(
                "Copied '{}' to '{}' with updated metadata from MusicBrainz",
                moved_filepath.display(),
                filepath.display()
            );

            Ok(None)
        }
    }
}

fn fetch_recording_data_from_mbz(
    entry: &LookupResultsEntry,
    runtime: &tokio::runtime::Handle,
) -> Vec<musicbrainz_rs::entity::recording::Recording> {
    fn entry_to_recording(
        entry: &RecordingEntry,
        runtime: &tokio::runtime::Handle,
    ) -> Result<musicbrainz_rs::entity::recording::Recording, musicbrainz_rs::Error> {
        runtime.block_on(
            musicbrainz_rs::entity::recording::Recording::fetch()
                .id(&entry.id)
                .with_artists()
                .execute(),
        )
    }

    let mut entries = entry.recordings.clone().unwrap_or_default();
    let mut recordings = Vec::new();

    'fetch: loop {
        entries.retain(|entry| match entry_to_recording(entry, runtime) {
            Ok(recording) => {
                recordings.push(recording);
                false
            }
            Err(_) => true,
        });
        if entries.is_empty() {
            break 'fetch;
        } else {
            let retry = dialoguer::Confirm::new()
                .with_prompt(format!(
                    "{} MusicBrainz API calls have failed, retry?",
                    entries.len()
                ))
                .default(true)
                .show_default(true)
                .wait_for_newline(true)
                .interact();

            match retry {
                Ok(true) => continue 'fetch,
                _ => break 'fetch,
            }
        }
    }

    recordings
}

pub(crate) fn artists_to_string(
    data: impl AsRef<[musicbrainz_rs::entity::artist_credit::ArtistCredit]>,
) -> String {
    let mut res = String::new();
    for artist in data.as_ref() {
        res.push_str(&artist.name);
        res.push(' ');
        if let Some(joinphrase) = &artist.joinphrase {
            res.push_str(joinphrase);
            res.push(' ');
        }
    }
    res
}

struct SelectionTreeLookupResultsEntry<'e> {
    entry: &'e LookupResultsEntry,
    _recording_data: once_cell::unsync::OnceCell<Vec<musicbrainz_rs::entity::recording::Recording>>,
    entry_display: String,
    _recording_display: once_cell::unsync::OnceCell<Vec<String>>,
}

impl<'e> SelectionTreeLookupResultsEntry<'e> {
    fn new(entry: &'e LookupResultsEntry) -> Self {
        Self {
            entry,
            _recording_data: once_cell::unsync::OnceCell::new(),
            entry_display: format!(
                "Score: {}, AcoustID: {}, Recordings: {}",
                &entry.score,
                &entry.id,
                entry.recordings.as_ref().unwrap().len()
            ),
            _recording_display: once_cell::unsync::OnceCell::new(),
        }
    }

    fn recording_data(
        &self,
        runtime: &tokio::runtime::Handle,
    ) -> &Vec<musicbrainz_rs::entity::recording::Recording> {
        self._recording_data
            .get_or_init(|| fetch_recording_data_from_mbz(self.entry, runtime))
    }

    fn recording_display(&self, runtime: &tokio::runtime::Handle) -> &Vec<String> {
        self._recording_display.get_or_init(|| {
            self.recording_data(runtime).iter().map(|recording| {
                format!(
                    "https://musicbrainz.org/recording/{}; Title: {}, Disambiguation: {}, Artists: {}",
                    &self.entry.id,
                    recording.title,
                    recording.disambiguation.as_ref().map(|s| s as &str).unwrap_or_else(|| ""),
                    artists_to_string(recording.artist_credit.as_ref().map(|s| s as &[_]).unwrap_or_else(|| &[]))
                )
            }).collect()
        })
    }
}

fn get_recording_from_selection_tree(
    results: &[LookupResultsEntry],
    runtime: tokio::runtime::Handle,
) -> Option<musicbrainz_rs::entity::recording::Recording> {
    let results: Vec<_> = results
        .iter()
        .map(SelectionTreeLookupResultsEntry::new)
        .collect();

    let results_display: Vec<_> = results
        .iter()
        .map(|tree_entry| &tree_entry.entry_display)
        .collect();

    let ask_top_level = |first_run: bool| {
        if first_run && results.len() == 1 {
            println!("Autoselecting {}", results_display[0]);
            Some(&results[0])
        } else {
            dialoguer::Select::new()
                .item("<none>")
                .items(&results_display)
                .default(0)
                .max_length(16)
                .interact()
                .ok()
                .filter(|index| *index > 0)
                .map(|index| &results[index - 1])
        }
    };

    let ask_results = |first_run: bool, entry: &SelectionTreeLookupResultsEntry| {
        let recordings = entry.recording_data(&runtime);
        let recordings_display = entry.recording_display(&runtime);

        if first_run && recordings.len() == 1 {
            println!("Autoselecting {}", recordings_display[0]);
            Some(recordings[0].clone())
        } else {
            dialoguer::Select::new()
                .with_prompt(format!("Select <back> to go back to the previous selection. Currently exploring AcoustID: {}", &entry.entry.id))
                .item("<back>")
                .items(recordings_display)
                .default(0)
                .max_length(16)
                .interact()
                .ok()
                .filter(|index| *index > 0)
                .map(|index| recordings[index - 1].clone())
        }
    };

    println!(
        "Explore the various associated recordings: Select <none> if none is correct, otherwise, when an option is selected, a nested selection for the correct MusicBrainz recording will appear"
    );

    let mut first_run = true;
    'outer: loop {
        match ask_top_level(first_run) {
            Some(entry) => 'inner: loop {
                match ask_results(first_run, entry) {
                    None => {
                        first_run = false;
                        continue 'outer;
                    }
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
                            first_run = false;
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
