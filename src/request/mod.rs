use crate::lock;
use crate::video;
use anyhow::anyhow;
use std::net::SocketAddr;
use std::time::Duration;

#[derive(clap::Args, Debug)]
pub(crate) struct RequestArgs {
    #[arg(
        long,
        help = "Youtube url to use for creating the video request. Supports the majority of modern youtube urls (will extract the ID)."
    )]
    pub yt_url: String,
}

pub(crate) async fn run(args: RequestArgs) -> Result<(), anyhow::Error> {
    let port = lock::read_port_no_lock()
        .await
        .expect("Failed to read daemon port from portfile! Is the daemon running?");

    let daemon_addr = format!("127.0.0.1:{}", port).parse::<SocketAddr>()?;
    let client = reqwest::Client::builder()
        .build()
        .expect("Failed to create http client!");

    println!("Creating video request...");
    let video_request = video::VideoRequest::from_yt_url(&args.yt_url)?;
    let yt_id = video_request.youtube_id.clone();

    println!("Sending request to daemon on {:?}", daemon_addr);
    let response = client
        .post(format!("http://127.0.0.1:{}/video-request", port))
        .form(&video_request)
        .timeout(Duration::from_secs(1))
        .send()
        .await?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(anyhow!(
            "Daemon ({daemon_addr:?}) for {yt_id} (http code: {}); {}",
            response.status(),
            response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unable to decode daemon response>")),
        ))
    }
}
