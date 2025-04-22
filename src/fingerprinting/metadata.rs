use crate::user::WhatToDo;
use crate::{cli, handle_what_to_do, musicbrainz, process};
use console::style;
use std::path::Path;
use std::sync::Arc;

pub(crate) async fn ffmpeg_modify_metadata_to_match_recording(
    filepath: &Path,
    recording: Arc<musicbrainz_rs::entity::recording::Recording>,
    args: &cli::TtyArgs,
) -> Result<Option<WhatToDo>, anyhow::Error> {
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
        &format!(
            "Artist={}",
            musicbrainz::artists_to_string(recording.artist_credit.as_ref().unwrap())
        ),
        "-codec",
        "copy",
        &filepath.display().to_string(),
    ];

    'last_command: loop {
        let ffmpeg_command_execution =
            process::handle_child_command_execution(&ffmpeg_cmd, movedir.path(), |cmd| cmd, process::wait_for_child)
                .await?
                .into_success_or_ask_wtd(|status, _unit| {
                    let message = format!("ffmpeg returned a non-zero exit code: {}", status);

                    (style(message).red(), WhatToDo::all())
                })
                .await?;

        match ffmpeg_command_execution {
            Ok(_unit) => {
                break 'last_command;
            }
            Err(what_to_do) => {
                handle_what_to_do!(what_to_do, [
                    retry: { continue 'last_command },
                    restart: { return Ok(Some(WhatToDo::RestartRequest)) },
                    cont: { break 'last_command },
                    abort: { return Ok(Some(WhatToDo::AbortRequest)) }
                ]);
            }
        }
    }

    println!(
        "{} '{}' to '{}' with updated metadata from MusicBrainz",
        style("Copied").yellow(),
        moved_filepath.display(),
        filepath.display()
    );

    Ok(None)
}
