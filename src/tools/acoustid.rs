use crate::tools::ChromaprintFingerprint;
use crate::tty::handle_requests::{artists_to_string, ask_action_on_command_error, WhatToDo};
use crate::tty::TtyArgs;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use musicbrainz_rs::Fetch;
use serde::{Deserialize, Serialize};
use std::iter::FusedIterator;
use std::time::Duration;

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
pub(crate) struct AcoustIDSubmission {
    status: String,
    submissions: Option<Vec<AcoustIDSubmissionEntry>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct AcoustIDSubmissionEntry {
    // why is this a string?
    index: Option<String>,
    id: u64,
    status: String,
}

pub async fn submit_fingerprint(
    acoustid_client: &mut reqwest::Client,
    fingerprint: &ChromaprintFingerprint,
    duration: u64,
    mbid: &str,
    user_api_key: &str,
    args: &TtyArgs,
) -> Result<(Option<WhatToDo>, musicbrainz_rs::entity::recording::Recording), anyhow::Error> {
    let recording = musicbrainz_rs::entity::recording::Recording::fetch()
        .id(mbid)
        .with_artists()
        .execute()
        .await?;

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

    let submission: AcoustIDSubmission = acoustid_client
        .post("https://api.acoustid.org/v2/submit")
        .query(&query)
        .send()
        .await?
        .json()
        .await?;

    let maybe_what_to_do = confirm_fingerprint_status(acoustid_client, submission, args).await?;

    Ok((maybe_what_to_do, recording))
}

struct RepeatLast<I, E> {
    inner: I,
    last: Option<E>,
}

impl<I, E: Clone> RepeatLast<I, E> {
    pub fn new(inner: I) -> Self {
        Self { inner, last: None }
    }
}

impl<I> Iterator for RepeatLast<I, <I as Iterator>::Item>
where
    I: Iterator,
    <I as Iterator>::Item: Clone,
{
    type Item = <I as Iterator>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next() {
            None => self.last.clone(),
            Some(item) => {
                self.last.replace(item.clone());
                Some(item)
            }
        }
    }

    // just like Cycle Iter
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.inner.size_hint() {
            sz @ (0, Some(0)) => sz,
            (0, _) => (0, None),
            _ => (usize::MAX, None),
        }
    }
}

impl<I> FusedIterator for RepeatLast<I, <I as Iterator>::Item>
where
    I: Iterator,
    <I as Iterator>::Item: Clone,
{
}

trait IntoRepeatLast<I, E> {
    fn repeat_last(self) -> RepeatLast<I, E>;
}

impl<I> IntoRepeatLast<I, <I as Iterator>::Item> for I
where
    I: Iterator,
    <I as Iterator>::Item: Clone,
{
    fn repeat_last(self) -> RepeatLast<I, <I as Iterator>::Item> {
        RepeatLast::new(self)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct AcoustIDSubmissionStatus {
    status: String,
    submissions: Option<Vec<AcoustIDSubmissionStatusEntry>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct AcoustIDSubmissionStatusEntry {
    id: u64,
    status: String,
    result: Option<AcoustIDSubmissionStatusEntryResult>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct AcoustIDSubmissionStatusEntryResult {
    id: String,
}

pub async fn confirm_fingerprint_status(
    acoustid_client: &mut reqwest::Client,
    submission: AcoustIDSubmission,
    args: &TtyArgs,
) -> Result<Option<WhatToDo>, anyhow::Error> {
    if submission.status != "ok" {
        return Ok(Some(
            ask_action_on_command_error(
                style(format!("AcoustID returned submission status {}.", submission.status)).red(),
                WhatToDo::all(),
            )
            .await?,
        ));
    }

    async fn wait(wait_time: u64) {
        const INTERVAL: u64 = 100;
        const MULTIPLIER: u64 = 1000 / INTERVAL;

        let bar = ProgressBar::new(wait_time * MULTIPLIER);
        bar.set_style(ProgressStyle::with_template("[{msg}] {wide_bar} {pos}/{len}").unwrap());
        bar.set_message(format!("Waiting {} seconds to get submit status...", wait_time));
        for _ in 0..(wait_time * MULTIPLIER) {
            // this is *good enough*
            tokio::time::sleep(Duration::from_millis(INTERVAL)).await;
            bar.inc(1);
        }
    }

    const WAIT_TIMES: [u64; 5] = [1, 2, 3, 5, 8];
    let mut wait_times = WAIT_TIMES.into_iter().repeat_last().enumerate();

    let submission_id = submission.submissions.as_ref().unwrap()[0].id;
    let submission_id_str = submission_id.to_string();
    let submission_acoustid_id = 'request_loop: loop {
        let (iteration, wait_time) = wait_times.next().unwrap();
        wait(wait_time).await;

        let submission_status: AcoustIDSubmissionStatus = acoustid_client
            .get("https://api.acoustid.org/v2/submission_status")
            .query(&[
                ("format", "json"),
                ("client", &args.acoustid_key),
                ("clientversion", env!("CARGO_PKG_VERSION")),
                ("id", &submission_id_str),
            ])
            .send()
            .await?
            .json()
            .await?;

        if submission_status.status != "ok" {
            if iteration > 3 {
                let what_to_do = ask_action_on_command_error(
                    style("AcoustID server keeps sending failed status response.".to_string()).red(),
                    WhatToDo::all(),
                )
                .await?;

                match what_to_do {
                    WhatToDo::Retry => continue 'request_loop,
                    WhatToDo::RestartRequest => return Ok(Some(WhatToDo::RestartRequest)),
                    WhatToDo::Continue => return Ok(Some(WhatToDo::Continue)),
                    WhatToDo::AbortRequest => return Ok(Some(WhatToDo::AbortRequest)),
                }
            }
            println!(
                "AcoustID server response status '{}', retrying...",
                submission_status.status
            );
            continue 'request_loop;
        }

        let entry_status = &submission_status.submissions.unwrap()[0];

        if entry_status.status != "imported" {
            if iteration > 4 {
                let what_to_do = ask_action_on_command_error(
                    style("AcoustID server keep sending not-'imported' submission status.".to_string()).red(),
                    WhatToDo::all(),
                )
                .await?;

                match what_to_do {
                    WhatToDo::Retry => continue 'request_loop,
                    WhatToDo::RestartRequest => return Ok(Some(WhatToDo::RestartRequest)),
                    WhatToDo::Continue => return Ok(Some(WhatToDo::Continue)),
                    WhatToDo::AbortRequest => return Ok(Some(WhatToDo::AbortRequest)),
                }
            }
            println!(
                "AcoustID submission entry status is '{}', retrying...",
                entry_status.status,
            );
            continue 'request_loop;
        }

        let submission_result = &entry_status.result.as_ref().unwrap().id;

        break 'request_loop submission_result.clone();
    };

    println!(
        "{}",
        style(format!(
            "AcoustID submission succeeded: https://acoustid.org/track/{}",
            &submission_acoustid_id
        ))
        .green()
    );

    // just to let user read
    let _ignore: String = tokio::task::spawn_blocking(move || {
        dialoguer::Input::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt(format!("Press {} to continue...", style("Enter").bold().cyan()))
            .allow_empty(true)
            .show_default(false)
            .report(false)
            .interact()
    })
    .await??;

    Ok(None)
}
