use crate::fingerprinting::acoustid::response::{LookupResultsEntry, RecordingEntry};
use console::style;
use musicbrainz_rs::Fetch;

pub(crate) fn fetch_recording_data_from_mbz(
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
