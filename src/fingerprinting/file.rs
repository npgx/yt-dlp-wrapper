use crate::fingerprinting::acoustid;
use crate::fingerprinting::acoustid::FingerprintSubmissionResult;
use crate::user::{ask_what_to_do, WhatToDo};
use crate::{cli, fingerprinting, handle_ctrlc, handle_what_to_do, process};
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

    handle_ctrlc!(restart: { return Ok(Some(WhatToDo::RestartRequest)) }, abort: { return Ok(Some(WhatToDo::AbortRequest)) });

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
    handle_ctrlc!(restart: { return Ok(Some(WhatToDo::RestartRequest)) }, abort: { return Ok(Some(WhatToDo::AbortRequest)) });

    let fpcalc_output = match fingerprint_filepath(filepath).await? {
        Ok(data) => data,
        Err(todo) => return Ok(Some(todo)),
    };

    handle_ctrlc!(restart: { return Ok(Some(WhatToDo::RestartRequest)) }, abort: { return Ok(Some(WhatToDo::AbortRequest)) });

    let fingerprint_lookup = 'lookup: loop {
        let lookup = acoustid::lookup_fingerprint(
            acoustid_client,
            &fpcalc_output.fingerprint,
            fpcalc_output.duration.floor() as u64,
            acoustid::ACOUSTID_CLIENT_KEY,
        )
        .await?;

        handle_ctrlc!(restart: { return Ok(Some(WhatToDo::RestartRequest)) }, abort: { return Ok(Some(WhatToDo::AbortRequest)) });

        match lookup.status.as_ref() {
            "ok" => break 'lookup Some(lookup),
            not_ok => {
                let what_to_do = ask_what_to_do(
                    style(format!(
                        "AcoustID fingerprint lookup failed! The fingerprint might not have been registered yet. Status: '{}'",
                        not_ok
                    )),
                    WhatToDo::all(),
                )
                .await?;

                handle_what_to_do!(what_to_do, [
                    retry: { continue 'lookup },
                    restart: { return Ok(Some(WhatToDo::RestartRequest)) },
                    cont: { break 'lookup None },
                    abort: { return Ok(Some(WhatToDo::AbortRequest)) }
                ]);
            }
        }
    };

    // Will be empty if lookup failed...
    let results = fingerprint_lookup
        .and_then(|lookup| lookup.results)
        .unwrap_or_else(Vec::new);

    // ...which will make both of these empty too...
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

    handle_ctrlc!(restart: { return Ok(Some(WhatToDo::RestartRequest)) }, abort: { return Ok(Some(WhatToDo::AbortRequest)) });

    if selection.is_none() {
        // ...which will trigger this
        match acoustid::handle_fingerprint_submission(acoustid_client, &fpcalc_output).await? {
            FingerprintSubmissionResult::Wtd(what_to_do) => return Ok(Some(what_to_do)),
            FingerprintSubmissionResult::Recording(recording) => {
                selection.replace(recording);
            }
            FingerprintSubmissionResult::Nothing => {
                // alright then
            }
        };
    }

    handle_ctrlc!(restart: { return Ok(Some(WhatToDo::RestartRequest)) }, abort: { return Ok(Some(WhatToDo::AbortRequest)) });

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

        let ffmpeg_command_execution = process::handle_child_command_execution(
            &fpcalc_cmd,
            path.parent().unwrap(),
            |mut cmd| {
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::piped());
                cmd
            },
            process::wait_for_child_output,
        )
        .await?
        .into_success_or_ask_wtd(|status, _output| {
            let message = format!("fpcalc returned a non-zero exit code: {}", status);

            (style(message).red(), WhatToDo::all_except(WhatToDo::Continue))
        })
        .await?;

        match ffmpeg_command_execution {
            Ok(output) => {
                break 'last_command output;
            }
            Err(what_to_do) => {
                handle_what_to_do!(what_to_do, [
                    retry: { continue 'last_command },
                    restart: { return Ok(Err(WhatToDo::RestartRequest)) },
                    cont: { unreachable!() },
                    abort: { return Ok(Err(WhatToDo::AbortRequest)) }
                ]);
            }
        }
    };

    let fpcalc_output: FPCalcJsonOutput = serde_json::from_slice(&output.stdout)?;

    Ok(Ok(fpcalc_output))
}
