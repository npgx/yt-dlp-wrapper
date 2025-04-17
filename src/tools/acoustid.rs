use crate::tools::chromaprint::ChromaprintFingerprint;

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
    fingerprint: ChromaprintFingerprint,
    track_duration: u64,
    client_api_key: &str,
) -> Result<response::Lookup, anyhow::Error> {
    let data: response::Lookup = client
        .post("https://api.acoustid.org/v2/lookup")
        .query(&[
            ("client", client_api_key),
            ("format", "json"),
            (
                "fingerprint",
                &fingerprint
                    .into_base64_urlsafe_fingerprint()
                    .into_result()?,
            ),
            ("meta", "recordings"),
            ("duration", &track_duration.to_string()),
        ])
        .send()
        .await?
        .json()
        .await?;

    Ok(data)
}
