use crate::BoxBodyBytes;
use flexbuffers::{DeserializationError, FlexbufferSerializer};
use http_body_util::{BodyExt, Empty};
use hyper::Response;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::sync::mpsc::Sender;
use url::Url;

pub(crate) async fn handle_video_http_request(
    request: hyper::Request<hyper::body::Incoming>,
    addr: SocketAddr,
    tx: Sender<VideoRequest>,
) -> Result<hyper::Response<BoxBodyBytes>, hyper::http::Error> {
    let vreq = VideoRequest::from_request(request, addr)
        .await
        .expect("Could not retrieve VideoRequest from HTTP request");

    tx.send(vreq)
        .await
        .expect("Could not enqueue video request into mpsc channel");

    Ok(Response::new(Box::new(
        Empty::new().map_err(|err| Box::new(err) as _),
    )))
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub(crate) struct VideoRequest {
    pub youtube_id: String,
}

#[derive(Debug)]
pub(crate) enum VideoRequestUrlParseError {
    #[allow(dead_code)]
    UnknownUrlKind(Url),
}

impl VideoRequest {
    pub fn from_yt_url(youtube_url: &str) -> Result<Self, VideoRequestUrlParseError> {
        let youtube_url: Url = youtube_url.parse().unwrap();
        let host_str = youtube_url.host_str().unwrap();

        let id: String =
            if host_str.ends_with("youtube.com") || host_str.ends_with("youtube-nocookie.com") {
                let segments = youtube_url.path_segments().unwrap().collect::<Vec<_>>();
                static SEGMENTS_2: [&str; 5] = ["watch", "v", "embed", "e", "shorts"];

                if segments.len() == 1 && segments[0] == "watch" {
                    // handle ...youtube.com/watch?v=XXXXXXXXXXX&foo=bar
                    let mut pairs = youtube_url.query_pairs();
                    let (_, v) = pairs.find(|(k, _)| k == "v").unwrap();
                    v.to_string()
                } else if segments.len() == 2 && SEGMENTS_2.contains(&segments[0]) {
                    // handle ...youtube.com/(watch|v)/XXXXXXXXXXX?foo=bar
                    segments[1].to_string()
                } else {
                    return Err(VideoRequestUrlParseError::UnknownUrlKind(youtube_url));
                }
            } else if youtube_url.host_str().unwrap().ends_with("youtu.be") {
                let segments = youtube_url.path_segments().unwrap().collect::<Vec<_>>();
                if segments.len() == 1 {
                    // handle ...youtu.be/XXXXXXXXXXX?foo=bar
                    segments[0].to_string()
                } else {
                    return Err(VideoRequestUrlParseError::UnknownUrlKind(youtube_url));
                }
            } else {
                // I got lazy: https://gist.github.com/rodrigoborgesdeoliveira/987683cfbfcc8d800192da1e73adc486
                return Err(VideoRequestUrlParseError::UnknownUrlKind(youtube_url));
            };

        Ok(Self { youtube_id: id })
    }

    pub async fn from_request(
        req: hyper::Request<hyper::body::Incoming>,
        addr: SocketAddr,
    ) -> Result<VideoRequest, DeserializationError> {
        let body_bytes = req.collect().await.unwrap().to_bytes();
        let s = flexbuffers::Reader::get_root(body_bytes.as_ref())?;
        VideoRequest::deserialize(s)
    }

    pub fn into_bytes(self) -> hyper::body::Bytes {
        let mut s = FlexbufferSerializer::new();
        self.serialize(&mut s).unwrap();
        hyper::body::Bytes::from(Vec::from(s.view()))
    }
}

impl Into<BoxBodyBytes> for VideoRequest {
    fn into(self) -> BoxBodyBytes {
        Box::new(http_body_util::Full::from(self.into_bytes()).map_err(|err| Box::new(err) as _))
    }
}
