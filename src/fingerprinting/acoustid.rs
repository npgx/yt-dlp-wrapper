use crate::fingerprinting::file::FPCalcJsonOutput;
use crate::musicbrainz::artists_to_string;
use crate::user::{ask_what_to_do, WhatToDo};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use musicbrainz_rs::Fetch;
use serde::{Deserialize, Serialize};
use std::iter::FusedIterator;
use std::time::Duration;

// this is not a secret, it's the API key of the client, which is this program
// if you're not convinced, here is picard's key:
// https://github.com/metabrainz/picard/commit/44c83e2ade75ea642a1b5ded7564262d5475977d
pub(crate) const ACOUSTID_CLIENT_KEY: &str = "bHEqneqDyO";

pub(crate) mod response {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Clone)]
    pub(crate) struct Lookup {
        pub(crate) status: String,
        pub(crate) results: Option<Vec<LookupResultsEntry>>,
    }

    #[derive(Serialize, Deserialize, Clone)]
    pub(crate) struct LookupResultsEntry {
        pub(crate) id: String,
        pub(crate) score: f64,
        pub(crate) recordings: Option<Vec<RecordingEntry>>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug)]
    pub(crate) struct RecordingEntry {
        pub(crate) id: String,
    }
}

pub(crate) async fn lookup_fingerprint(
    client: &mut reqwest::Client,
    fingerprint: &str,
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

pub(crate) async fn submit_fingerprint(
    acoustid_client: &mut reqwest::Client,
    fingerprint: &str,
    duration: u64,
    mbid: &str,
    user_api_key: &str,
) -> Result<(Option<WhatToDo>, musicbrainz_rs::entity::recording::Recording), anyhow::Error> {
    let recording = musicbrainz_rs::entity::recording::Recording::fetch()
        .id(mbid)
        .with_artists()
        .execute()
        .await?;

    let duration = duration.to_string();
    let mut query = vec![
        ("format", "json"),
        ("client", ACOUSTID_CLIENT_KEY),
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

    let maybe_what_to_do = confirm_fingerprint_status(acoustid_client, submission).await?;

    Ok((maybe_what_to_do, recording))
}

struct RepeatLast<I, E> {
    inner: I,
    last: Option<E>,
}

impl<I, E: Clone> RepeatLast<I, E> {
    pub(crate) fn new(inner: I) -> Self {
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

pub(crate) enum FingerprintSubmissionResult {
    WTD(WhatToDo),
    Recording(musicbrainz_rs::entity::recording::Recording),
    Nothing,
}

pub(crate) async fn handle_fingerprint_submission(
    acoustid_client: &mut reqwest::Client,
    fpcalc_output: &FPCalcJsonOutput,
) -> Result<FingerprintSubmissionResult, anyhow::Error> {
    use once_cell::sync::OnceCell;
    use std::borrow::Cow;
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

            let (what_to_do, recording) = submit_fingerprint(
                acoustid_client,
                &fpcalc_output.fingerprint,
                fpcalc_output.duration.floor() as u64,
                &mbid,
                &acoustid_user_key,
            )
            .await?;

            match what_to_do {
                Some(WhatToDo::Retry) => continue 'submit,
                Some(WhatToDo::RestartRequest) => return Ok(FingerprintSubmissionResult::WTD(WhatToDo::AbortRequest)),
                None => {
                    println!(
                        "{}",
                        style("Persisting AcoustID User Key (for current session)").magenta()
                    );
                    ACOUSTID_USER_KEY.get_or_init(|| acoustid_user_key.into_owned());
                    return Ok(FingerprintSubmissionResult::Recording(recording));
                }
                Some(WhatToDo::Continue) => {
                    return Ok(FingerprintSubmissionResult::Recording(recording));
                }
                Some(WhatToDo::AbortRequest) => return Ok(FingerprintSubmissionResult::WTD(WhatToDo::AbortRequest)),
            }
        }
    }

    Ok(FingerprintSubmissionResult::Nothing)
}

pub(crate) async fn confirm_fingerprint_status(
    acoustid_client: &mut reqwest::Client,
    submission: AcoustIDSubmission,
) -> Result<Option<WhatToDo>, anyhow::Error> {
    if submission.status != "ok" {
        return Ok(Some(
            ask_what_to_do(
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
                ("client", ACOUSTID_CLIENT_KEY),
                ("clientversion", env!("CARGO_PKG_VERSION")),
                ("id", &submission_id_str),
            ])
            .send()
            .await?
            .json()
            .await?;

        if submission_status.status != "ok" {
            if iteration > 3 {
                let what_to_do = ask_what_to_do(
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
                let what_to_do = ask_what_to_do(
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
