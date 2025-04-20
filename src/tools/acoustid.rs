use crate::tools::ChromaprintFingerprint;
use crate::tty::handle_requests::{artists_to_string, ask_action_on_command_error, WhatToDo};
use crate::tty::TtyArgs;
use musicbrainz_rs::Fetch;
use serde::{Deserialize, Serialize};

pub mod response {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Clone)]
    pub struct Lookup {
        pub status: String,
        pub results: Option<Vec<LookupResultsEntry>>,
    }

    #[derive(Serialize, Deserialize, Clone)]
    pub struct LookupResultsEntry {
        pub id: String,
        pub score: f64,
        pub recordings: Option<Vec<RecordingEntry>>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug)]
    pub struct RecordingEntry {
        pub id: String,
    }
}

pub async fn lookup_fingerprint(
    client: &mut reqwest::Client,
    fingerprint: &ChromaprintFingerprint,
    track_duration: u64,
    client_api_key: &str,
) -> Result<response::Lookup, anyhow::Error> {
    let data: response::Lookup = client
        .post("https://api.acoustid.org/v2/lookup")
        .query(&[
            ("client", client_api_key),
            ("format", "json"),
            ("fingerprint", fingerprint),
            ("meta", "recordings"),
            ("duration", &track_duration.to_string()),
        ])
        .send()
        .await?
        .json()
        .await?;

    Ok(data)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AcoustIDSubmission {
    status: String,
    submissions: Option<Vec<AcoustIDSubmissionEntry>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AcoustIDSubmissionEntry {
    index: Option<u32>,
    id: u64,
    status: String,
}

pub async fn submit_fingerprint(
    client: &mut reqwest::Client,
    fingerprint: &ChromaprintFingerprint,
    duration: u64,
    mbid: &str,
    user_api_key: &str,
    args: &TtyArgs,
) -> Result<
    (
        Option<WhatToDo>,
        musicbrainz_rs::entity::recording::Recording,
    ),
    anyhow::Error,
> {
    let recording = musicbrainz_rs::entity::recording::Recording::fetch()
        .id(mbid)
        .with_artists()
        .execute()
        .await?;

    /*let metadata_path = filepath.with_extension("metadata.json");
    let ffmpeg_metadata_cmd = [
        "ffprobe",
        "-loglevel",
        &args.ffmpeg_loglevel,
        "-i",
        &filepath.display().to_string(),
        "-print_format",
        "json=compact",
        "-show_format",
        "-show_streams",
        &metadata_path.display().to_string(),
    ];

    'metadata: loop {
        let status = wrap_command_print_context(
            &ffmpeg_metadata_cmd,
            filepath.parent().unwrap(),
            |cmd| cmd,
            wait_for_cmd,
        )
        .await?;

        if !status.exit_status.success() {
            match tty::handle_requests::ask_action_on_command_error(true).await? {
                WhatToDo::RetryLastCommand => continue 'metadata,
                WhatToDo::Continue => break 'metadata,
                WhatToDo::RestartRequest => return Ok(Some(WhatToDo::RestartRequest)),
                WhatToDo::AbortRequest => return Ok(Some(WhatToDo::AbortRequest)),
            }
        }
    }*/

    let duration = duration.to_string();
    let mut query = vec![
        ("format", "json"),
        ("client", &args.acoustid_key),
        ("clientversion", env!("CARGO_PKG_VERSION")),
        ("user", user_api_key),
        ("duration.0", &duration),
        ("fingerprint.0", fingerprint),
        ("mbid.0", mbid),
        ("track.0", &recording.title),
    ];
    let artists = recording.artist_credit.as_ref().map(artists_to_string);
    if let Some(artists) = &artists {
        query.push(("artist.0", artists));
    };

    let response: AcoustIDSubmission = client
        .post("https://api.acoustid.org/v2/submit")
        .query(&query)
        .send()
        .await?
        .json()
        .await?;

    println!("AcoustID Submission Response: {:#?}", response);

    Ok((
        Some(ask_action_on_command_error(String::from(""), true).await?),
        recording,
    ))
}
