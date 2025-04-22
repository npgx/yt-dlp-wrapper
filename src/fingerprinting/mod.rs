pub(crate) mod acoustid;
pub(crate) mod file;
pub(crate) mod metadata;

use crate::fingerprinting::acoustid::response::LookupResultsEntry;
use crate::musicbrainz;
use console::style;
use std::future::ready;
use std::sync::Arc;

struct SelectionTreeLookupResultsEntry<'lre> {
    entry: &'lre LookupResultsEntry,
    _recording_data: tokio::sync::OnceCell<Vec<Arc<musicbrainz_rs::entity::recording::Recording>>>,
    entry_display: String,
    _recording_display: tokio::sync::OnceCell<Arc<Vec<String>>>,
}

impl<'lre> SelectionTreeLookupResultsEntry<'lre> {
    fn new(entry: &'lre LookupResultsEntry) -> Self {
        Self {
            _recording_data: tokio::sync::OnceCell::new(),
            entry_display: format!(
                "Score: {}, AcoustID: {}, Recordings: {}",
                style(&entry.score).cyan().bold(),
                &entry.id,
                style(entry.recordings.as_ref().unwrap().len()).cyan()
            ),
            _recording_display: tokio::sync::OnceCell::new(),
            entry,
        }
    }

    pub(crate) async fn recording_data(&self) -> &Vec<Arc<musicbrainz_rs::entity::recording::Recording>> {
        self._recording_data
            .get_or_init(|| async {
                match self.entry.recordings.as_ref() {
                    Some(vec) => {
                        musicbrainz::fetch_all_recordings_with_interact(
                            vec.iter().map(|entry| &entry.id).collect::<Vec<_>>(),
                        )
                        .await
                    }
                    None => ready(Vec::new()).await,
                }
            })
            .await
    }

    pub(crate) async fn recording_display(&self) -> Arc<Vec<String>> {
        self._recording_display
            .get_or_init(|| async {
                Arc::new(
                    self.recording_data()
                        .await
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
                                style(musicbrainz::artists_to_string(
                                    recording
                                        .artist_credit
                                        .as_ref()
                                        .map(|s| s as &[_])
                                        .unwrap_or_else(|| &[])
                                ))
                                .blue()
                            )
                        })
                        .collect(),
                )
            })
            .await
            .clone()
    }
}

mod tree {
    use super::*;

    pub(super) async fn ask_top_level<'lre>(
        first_run: bool,
        results: &'lre [SelectionTreeLookupResultsEntry<'lre>],
        results_display: Arc<Vec<String>>,
    ) -> Result<Option<&'lre SelectionTreeLookupResultsEntry<'lre>>, tokio::task::JoinError> {
        if first_run && results.len() == 1 && results[0].entry.score > 0.95 {
            println!("{} {}", style("Autoselecting").magenta(), &results_display[0]);
            Ok(Some(&results[0]))
        } else {
            let selected = tokio::task::spawn_blocking(move || {
                dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
                    .item("<none>")
                    .items(&results_display)
                    .default(0)
                    .max_length(16)
                    .interact()
                    .ok()
                    .filter(|index| *index > 0)
            })
            .await?;

            Ok(selected.map(|index| &results[index - 1]))
        }
    }

    pub(super) async fn ask_results<'lre>(
        first_run: bool,
        entry: &'lre SelectionTreeLookupResultsEntry<'lre>,
    ) -> Result<Option<Arc<musicbrainz_rs::entity::recording::Recording>>, tokio::task::JoinError> {
        let recordings = entry.recording_data().await;
        let recordings_display = entry.recording_display().await;

        if first_run && recordings.len() == 1 {
            println!("{} {}", style("Autoselecting").magenta(), recordings_display[0]);
            Ok(Some(recordings[0].clone()))
        } else {
            let id = entry.entry.id.clone();
            let selected = tokio::task::spawn_blocking(move || {
                dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
                    .with_prompt(format!("{}: {}", style("Currently exploring AcoustID").italic(), id))
                    .item("<back>")
                    .items(&recordings_display)
                    .default(0)
                    .max_length(16)
                    .interact()
                    .ok()
                    .filter(|index| *index > 0)
            })
            .await?;

            Ok(selected.map(|index| recordings[index - 1].clone()))
        }
    }
}

async fn get_recording_from_selection_tree(
    results: &[LookupResultsEntry],
) -> Result<Option<Arc<musicbrainz_rs::entity::recording::Recording>>, anyhow::Error> {
    let results: Vec<SelectionTreeLookupResultsEntry> =
        results.iter().map(SelectionTreeLookupResultsEntry::new).collect();

    let results_display: Arc<Vec<String>> = Arc::new(
        results
            .iter()
            .map(|tree_entry| tree_entry.entry_display.clone())
            .collect(),
    );

    println!(
        "Select correct recording, or {} if none is correct",
        style("<none>").bold(),
    );

    let mut first_run = true;
    'outer: loop {
        match tree::ask_top_level(first_run, &results, results_display.clone()).await? {
            Some(entry) => 'inner: loop {
                match tree::ask_results(first_run, entry).await? {
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
                            style(
                                &record
                                    .artist_credit
                                    .as_ref()
                                    .map(musicbrainz::artists_to_string)
                                    .unwrap_or_default()
                            )
                            .cyan()
                            .bold(),
                        );
                        let confirm = tokio::task::spawn_blocking(move || {
                            dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                                .with_prompt("Confirm?")
                                .default(true)
                                .show_default(true)
                                .wait_for_newline(true)
                                .interact()
                                .is_ok_and(|r| r)
                        })
                        .await?;

                        if confirm {
                            return Ok(Some(record));
                        } else {
                            first_run = false;
                            continue 'inner;
                        }
                    }
                }
            },
            None => return Ok(None),
        }
    }
}
