use crate::fingerprinting::acoustid;
use crate::fingerprinting::acoustid::FingerprintSubmissionResult;
use crate::user::WhatToDo;
use crate::{cli, fingerprinting, process};
use console::style;
use std::path::Path;

pub(crate) fn get_fingerprintable_filenames_in_directory(path: &Path) -> Vec<String> {
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

pub(crate) async fn handle_fingerprinting_process_for_directory(
    work_dir: &Path,
    acoustid_client: &mut reqwest::Client,
    args: &cli::TtyArgs,
) -> Result<Option<WhatToDo>, anyhow::Error> {
    let fingerprintable = get_fingerprintable_filenames_in_directory(work_dir);

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
        println!(
            "{}",
            style("<none> selected, ignoring other selections!").magenta().bold()
        );
        selections.clear();
    }

    let mut to_fingerprint = Vec::with_capacity(selections.len());
    for selection in selections {
        to_fingerprint.push(&fingerprintable[selection - 1]);
    }

    for filename in to_fingerprint {
        let filepath = work_dir.join(filename);
        if let Some(todo) = handle_fingerprinting_process_for_filepath(&filepath, acoustid_client, args).await? {
            return Ok(Some(todo));
        }
    }

    Ok(None)
}

pub(crate) async fn handle_fingerprinting_process_for_filepath(
    filepath: &Path,
    acoustid_client: &mut reqwest::Client,
    args: &cli::TtyArgs,
) -> Result<Option<WhatToDo>, anyhow::Error> {
    let fpcalc_output = match fingerprint_filepath(filepath).await? {
        Ok(data) => data,
        Err(todo) => return Ok(Some(todo)),
    };

    let fingerprint_lookup = acoustid::lookup_fingerprint(
        acoustid_client,
        &fpcalc_output.fingerprint,
        fpcalc_output.duration.floor() as u64,
        acoustid::ACOUSTID_CLIENT_KEY,
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

    let mut selection = if results_with_recordings.is_empty() {
        println!("{}", style("No AcoustID matches with associated recordings!").magenta());
        None
    } else {
        fingerprinting::get_recording_from_selection_tree(&results_with_recordings).await?
    };

    if selection.is_none() {
        match acoustid::handle_fingerprint_submission(acoustid_client, &fpcalc_output).await? {
            FingerprintSubmissionResult::WTD(what_to_do) => return Ok(Some(what_to_do)),
            FingerprintSubmissionResult::Recording(recording) => {
                selection.replace(recording);
            }
            FingerprintSubmissionResult::Nothing => {
                // alright then
            }
        };
    }

    match selection {
        None => Ok(None),
        Some(recording) => {
            let maybe_what_to_do =
                fingerprinting::metadata::ffmpeg_modify_metadata_to_match_recording(filepath, recording, args).await?;
            Ok(maybe_what_to_do)
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct FPCalcJsonOutput {
    pub(crate) duration: f64,
    pub(crate) fingerprint: String,
}

pub(crate) async fn fingerprint_filepath(path: &Path) -> Result<Result<FPCalcJsonOutput, WhatToDo>, anyhow::Error> {
    let mut fpcalc_cmd = vec![String::from("fpcalc"), String::from("-json")];
    fpcalc_cmd.push(path.display().to_string());

    let output = 'last_command: loop {
        use std::process::Stdio;

        let output = process::wrap_command_print_context(
            &fpcalc_cmd,
            path.parent().unwrap(),
            |mut cmd| {
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::piped());
                cmd
            },
            process::wait_for_child_output,
        )
        .await?;

        if !output.exit_status.success() {
            match crate::user::ask_what_to_do(
                style(format!("fpcalc returned a non-zero exit code: {}", output.exit_status)).red(),
                WhatToDo::all_except(WhatToDo::Continue),
            )
            .await?
            {
                WhatToDo::Retry => continue 'last_command,
                WhatToDo::Continue => panic!(),
                WhatToDo::RestartRequest => return Ok(Err(WhatToDo::RestartRequest)),
                WhatToDo::AbortRequest => return Ok(Err(WhatToDo::AbortRequest)),
            }
        }

        break 'last_command output.data;
    };

    let fpcalc_output: FPCalcJsonOutput = serde_json::from_slice(&output.stdout)?;

    Ok(Ok(fpcalc_output))
}
