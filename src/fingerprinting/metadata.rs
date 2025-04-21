use crate::user::WhatToDo;
use crate::{cli, musicbrainz, process, user};
use console::style;
use std::path::Path;

pub(crate) async fn ffmpeg_modify_metadata_to_match_recording(
    filepath: &Path,
    recording: musicbrainz_rs::entity::recording::Recording,
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
            musicbrainz::artists_to_string(recording.artist_credit.unwrap())
        ),
        "-codec",
        "copy",
        &filepath.display().to_string(),
    ];

    'last_command: loop {
        let ffmpeg_exit_status =
            process::wrap_command_print_context(&ffmpeg_cmd, movedir.path(), |cmd| cmd, process::wait_for_cmd).await?;

        if !ffmpeg_exit_status.exit_status.success() {
            match user::ask_what_to_do(
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
