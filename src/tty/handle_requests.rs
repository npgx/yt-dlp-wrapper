use crate::tools::acoustid;
use crate::tools::acoustid::response::{LookupResultsEntry, RecordingEntry};
use crate::tty::{PromptFlag, WithExitStatus};
use crate::{tools, tty};
use console::style;
use musicbrainz_rs::Fetch;
use std::borrow::Cow;
use std::fmt::{Display, Formatter};
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
            let yt_dlp_exit_status =
                tty::wrap_command_print_context(&ytdlp_cmd, work_dir_path, |cmd| cmd, tty::wait_for_cmd).await?;

            if !yt_dlp_exit_status.exit_status.success() {
                match ask_action_on_command_error(
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
            if let Some(todo) = handle_workdir_fingerprinting(work_dir_path, acoustid_client, args).await? {
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
                tty::wrap_command_print_context(&beet_cmd, work_dir_path, |cmd| cmd, tty::wait_for_cmd).await?;

            if !beet_exit_status.exit_status.success() {
                match ask_action_on_command_error(
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
            PromptFlag::Always => true,
            PromptFlag::Never => false,
            PromptFlag::Ask => {
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

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
pub(crate) enum WhatToDo {
    Retry,
    RestartRequest,
    Continue,
    AbortRequest,
}

impl WhatToDo {
    pub(crate) const fn all() -> &'static [Self] {
        &[
            WhatToDo::Retry,
            WhatToDo::RestartRequest,
            WhatToDo::Continue,
            WhatToDo::AbortRequest,
        ]
    }

    pub(crate) const fn all_except(except: WhatToDo) -> &'static [Self] {
        match except {
            WhatToDo::Retry => &[WhatToDo::RestartRequest, WhatToDo::Continue, WhatToDo::AbortRequest],
            WhatToDo::RestartRequest => &[WhatToDo::Retry, WhatToDo::Continue, WhatToDo::AbortRequest],
            WhatToDo::Continue => &[WhatToDo::Retry, WhatToDo::RestartRequest, WhatToDo::AbortRequest],
            WhatToDo::AbortRequest => &[WhatToDo::Retry, WhatToDo::RestartRequest, WhatToDo::Continue],
        }
    }
}

impl Display for WhatToDo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            WhatToDo::Retry => {
                write!(f, "Retry")
            }
            WhatToDo::RestartRequest => {
                write!(f, "Restart video request")
            }
            WhatToDo::Continue => {
                write!(f, "Continue...")
            }
            WhatToDo::AbortRequest => {
                write!(f, "Abort the video request")
            }
        }
    }
}

pub(crate) async fn ask_action_on_command_error(
    message: console::StyledObject<String>,
    allowed: impl AsRef<[WhatToDo]>,
) -> Result<WhatToDo, anyhow::Error> {
    let mut allowed = allowed.as_ref().to_vec();
    allowed.sort();

    if allowed.is_empty() {
        panic!("Internal Error: ask_action_on_command_error received empty 'allowed'")
    }

    let todo = tokio::task::spawn_blocking(move || {
        dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt(format!("{message}\n{}", style("What would you like to do?").cyan()))
            .default(0)
            .items(&allowed)
            .interact()
            .map(|ordinal| allowed[ordinal])
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
        dialoguer::MultiSelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt(format!(
                "Select files to fingerprint, if {} is selected, {}",
                style("<none>").bold(),
                style("all other selections will be ignored").italic().red()
            ))
            .item("<none>")
            .items(&selections_fingerprintable)
            .defaults(&defaults)
            .max_length(16)
            .interact()
    })
    .await??;

    // is <none> selected
    if selections.contains(&0) {
        println!("{}", style("Fingerprinting disabled!").magenta().bold());
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

    let (results_with_recordings, results_others): (Vec<_>, Vec<_>) = results
        .into_iter()
        .partition(|entry| entry.recordings.as_ref().is_some_and(|recs| !recs.is_empty()));

    if !results_others.is_empty() {
        println!(
            "{}",
            style(format!(
                "Ignoring {} matches that do not have any associated recordings!",
                results_others.len()
            ))
            .yellow()
        );
    }

    let runtime = tokio::runtime::Handle::current();

    let mut selection = if results_with_recordings.is_empty() {
        println!("{}", style("No AcoustID matches with associated recordings!").magenta());
        None
    } else {
        tokio::task::spawn_blocking(move || get_recording_from_selection_tree(&results_with_recordings, runtime))
            .await?
    };

    if selection.is_none() {
        use once_cell::sync::OnceCell;
        static ACOUSTID_USER_KEY: OnceCell<String> = OnceCell::new();

        let submit = tokio::task::spawn_blocking(move || {
            dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt(format!("{}", style("Would you like to submit the fingerprint?").cyan()))
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
                            dialoguer::Input::<String>::with_theme(&dialoguer::theme::ColorfulTheme::default())
                                .with_prompt("Insert the AcoustID user API key (https://acoustid.org)")
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
                        dialoguer::Input::<String>::with_theme(&dialoguer::theme::ColorfulTheme::default())
                            .with_prompt(
                                "Insert the MusicBrainz RECORDING ID that you would like to bind to the fingerprint",
                            )
                            .allow_empty(false)
                            .report(true)
                            .interact_text()
                    })
                    .await?;
                    match tmp {
                        Ok(value) => {
                            let value2 = value.clone();
                            let confirm = tokio::task::spawn_blocking(move || {
                                dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                                    .with_prompt(format!(
                                        "{}: {}",
                                        style("Confirm MusicBrainz RECORDING ID").green(),
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
                        Err(err) => {
                            eprintln!("{}: {}", style("Invalid Record ID").for_stderr().red(), err)
                        }
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
                    Some(WhatToDo::Retry) => continue 'submit,
                    Some(WhatToDo::RestartRequest) => return Ok(Some(WhatToDo::AbortRequest)),
                    None => {
                        println!(
                            "{}",
                            style("Persisting AcoustID User Key (for current session)").magenta()
                        );
                        ACOUSTID_USER_KEY.get_or_init(|| acoustid_user_key.into_owned());
                        selection.replace(recording);
                        break 'submit;
                    }
                    Some(WhatToDo::Continue) => {
                        selection.replace(recording);
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
                "{} '{}' to '{}'",
                style("Moving").yellow(),
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
                &format!("Artist={}", artists_to_string(recording.artist_credit.unwrap())),
                "-codec",
                "copy",
                &filepath.display().to_string(),
            ];

            'last_command: loop {
                let ffmpeg_exit_status =
                    tty::wrap_command_print_context(&ffmpeg_cmd, movedir.path(), |cmd| cmd, tty::wait_for_cmd).await?;

                if !ffmpeg_exit_status.exit_status.success() {
                    match ask_action_on_command_error(
                        style(format!(
                            "ffmpeg returned a non-zero exit code: {}",
                            ffmpeg_exit_status.exit_status
                        ))
                        .red(),
                        WhatToDo::all(),
                    )
                    .await?
                    {
                        WhatToDo::Retry => continue 'last_command,
                        WhatToDo::RestartRequest => return Ok(Some(WhatToDo::RestartRequest)),
                        WhatToDo::Continue => break 'last_command,
                        WhatToDo::AbortRequest => return Ok(Some(WhatToDo::AbortRequest)),
                    }
                }

                break 'last_command;
            }

            println!(
                "{} '{}' to '{}' with updated metadata from MusicBrainz",
                style("Copied").yellow(),
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
            let retry = dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt(format!(
                    "{} {}, retry?",
                    style(entries.len()).red(),
                    style("MusicBrainz API calls have failed").red(),
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

pub(crate) fn artists_to_string(data: impl AsRef<[musicbrainz_rs::entity::artist_credit::ArtistCredit]>) -> String {
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
                style(&entry.score).cyan().bold(),
                &entry.id,
                style(entry.recordings.as_ref().unwrap().len()).cyan()
            ),
            _recording_display: once_cell::unsync::OnceCell::new(),
        }
    }

    fn recording_data(&self, runtime: &tokio::runtime::Handle) -> &Vec<musicbrainz_rs::entity::recording::Recording> {
        self._recording_data
            .get_or_init(|| fetch_recording_data_from_mbz(self.entry, runtime))
    }

    fn recording_display(&self, runtime: &tokio::runtime::Handle) -> &Vec<String> {
        self._recording_display.get_or_init(|| {
            self.recording_data(runtime)
                .iter()
                .map(|recording| {
                    format!(
                        "https://musicbrainz.org/recording/{}; Title: {}, Disambiguation: {}, Artists: {}",
                        &self.entry.id,
                        style(&recording.title).blue(),
                        style(
                            recording
                                .disambiguation
                                .as_ref()
                                .map(|s| s as &str)
                                .unwrap_or_else(|| "")
                        )
                        .blue(),
                        style(artists_to_string(
                            recording
                                .artist_credit
                                .as_ref()
                                .map(|s| s as &[_])
                                .unwrap_or_else(|| &[])
                        ))
                        .blue()
                    )
                })
                .collect()
        })
    }
}

fn get_recording_from_selection_tree(
    results: &[LookupResultsEntry],
    runtime: tokio::runtime::Handle,
) -> Option<musicbrainz_rs::entity::recording::Recording> {
    let results: Vec<_> = results.iter().map(SelectionTreeLookupResultsEntry::new).collect();

    let results_display: Vec<_> = results.iter().map(|tree_entry| &tree_entry.entry_display).collect();

    let ask_top_level = |first_run: bool| {
        if first_run && results.len() == 1 && results[0].entry.score > 0.95 {
            println!("{} {}", style("Autoselecting").magenta(), results_display[0]);
            Some(&results[0])
        } else {
            dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
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
            println!("{} {}", style("Autoselecting").magenta(), recordings_display[0]);
            Some(recordings[0].clone())
        } else {
            dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt(format!(
                    "{}: {}",
                    style("Currently exploring AcoustID").italic(),
                    &entry.entry.id
                ))
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
        "Select correct recording, or {} if none is correct",
        style("<none>").bold(),
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
                        let empty_string = String::new();
                        println!(
                            "\n{}\nRecording: https://musicbrainz.org/recording/{}\nTitle: {}\nDisambiguation: {}\nArtists: {}\n",
                            style("Selected:").blue().bold(),
                            &record.id,
                            style(&record.title).cyan().bold(),
                            style(&record.disambiguation.as_ref().unwrap_or(&empty_string))
                                .cyan()
                                .bold(),
                            style(&record.artist_credit.as_ref().map(artists_to_string).unwrap_or_default())
                                .cyan()
                                .bold(),
                        );
                        let confirm = dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                            .with_prompt("Confirm?")
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
