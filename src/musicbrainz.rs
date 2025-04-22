use console::style;
use musicbrainz_rs::Fetch;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) async fn fetch_recording_data(
    mbid: impl AsRef<str>,
) -> Result<Arc<musicbrainz_rs::entity::recording::Recording>, musicbrainz_rs::Error> {
    musicbrainz_rs::entity::recording::Recording::fetch()
        .id(mbid.as_ref())
        .with_artists()
        .execute()
        .await
        .map(Arc::new)
}

pub(crate) async fn fetch_all_recordings_with_interact<A, S>(
    mbids: A,
) -> Vec<Arc<musicbrainz_rs::entity::recording::Recording>>
where
    A: AsRef<[S]>,
    S: AsRef<str> + Clone,
{
    // shouldn't, but might, have duplicates
    let mbids = mbids.as_ref();

    // Option<Arc<_>> lets us check if we're all done below,
    // without having to check if all elements in mbids are in the cache
    let mut cache =
        HashMap::<&str, Option<Arc<musicbrainz_rs::entity::recording::Recording>>>::with_capacity(mbids.len());

    // initialize it to None
    for mbid in mbids {
        cache.insert(mbid.as_ref(), None);
    }

    'interact: loop {
        for mbid in mbids {
            let mbid = mbid.as_ref();

            match cache.get(mbid) {
                None => unreachable!(),
                Some(None) => match fetch_recording_data(mbid).await {
                    Ok(recording) => {
                        cache.insert(mbid, Some(recording));
                    }
                    Err(err) => {
                        println!("Failed to fetch https://musicbrainz.org/recording/{}: {}", mbid, err);
                    }
                },
                Some(_) => {}
            }
        }

        if cache.values().all(Option::is_some) {
            break 'interact;
        } else {
            let retry = dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt(format!(
                    "{} {}, retry?",
                    style(cache.values().filter(|opt| opt.is_none()).count()).red(),
                    style("MusicBrainz API calls have failed").red(),
                ))
                .default(true)
                .show_default(true)
                .wait_for_newline(true)
                .interact();

            match retry {
                Ok(true) => continue 'interact,
                _ => break 'interact,
            }
        }
    }

    cache.into_values().flatten().collect()
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
