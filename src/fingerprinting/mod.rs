pub(crate) mod acoustid;
pub(crate) mod file;
pub(crate) mod metadata;

use crate::fingerprinting::acoustid::response::LookupResultsEntry;
use crate::musicbrainz;
use crate::process::WithExitStatus;
use console::style;
use std::process::ExitStatus;

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
            .get_or_init(|| musicbrainz::fetch_recording_data_from_mbz(self.entry, runtime))
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
