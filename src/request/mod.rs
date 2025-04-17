use crate::lock;
use crate::video;
use std::net::SocketAddr;

#[derive(clap::Args, Debug)]
pub(crate) struct RequestArgs {
    #[arg(
        long,
        help = "Youtube url to use for creating the video request. Supports the majority of modern youtube urls (will extract the ID)."
    )]
    pub yt_url: String,
}

pub(crate) async fn run(args: RequestArgs) {
    let port = lock::read_port_no_lock()
        .await
        .expect("Failed to read daemon port from portfile! Is the daemon running?");

    let daemon_addr = format!("127.0.0.1:{}", port).parse::<SocketAddr>().unwrap();
    let client = reqwest::Client::builder()
        .build()
        .expect("Failed to create http client!");

    println!("Sending request to daemon on {:?}", daemon_addr);
    let video_request =
        video::VideoRequest::from_yt_url(&args.yt_url).expect("Failed to create video request!");
    let yt_id = video_request.youtube_id.clone();

    let response = client
        .post(format!("http://127.0.0.1:{}", port))
        .body(video_request.into_bytes())
        .send()
        .await
        .expect("Failed to send http request to daemon!");

    let status = response.status();
    response.error_for_status()
        .expect(&format!("Video request for {yt_id} has been sent to daemon on {daemon_addr:?}, but daemon responded with error code {:?}!", status));
}
