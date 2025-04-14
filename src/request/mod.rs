use crate::video;
use crate::{client, lock};
use http_body_util::BodyExt;
use hyper::StatusCode;
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

    let mut daemon_connection = client::connect_to_daemon(daemon_addr)
        .await
        .expect("Failed to connect to daemon!");

    println!("Connected to daemon on {:?}", daemon_addr);

    let video_request =
        video::VideoRequest::from_yt_url(&args.yt_url).expect("Failed to create video request!");
    let yt_id = video_request.youtube_id.clone();

    let mut response = client::send_video_request_to_daemon(&mut daemon_connection, video_request)
        .await
        .expect("Failed to send video request to daemon!");

    let mut data_buf = Vec::new();

    while let Some(next) = response.frame().await {
        let frame = next.expect("Failed to get frame from daemon response!");
        if let Some(data) = frame.data_ref() {
            data_buf.extend_from_slice(data);
        }
    }

    match response.status() {
        StatusCode::BAD_REQUEST => {
            eprintln!(
                "ERROR: Daemon on {:?} responded to video request for {} with status code 400 (BAD_REQUEST)! Response body: '{}'",
                daemon_addr,
                yt_id,
                String::from_utf8_lossy(&data_buf)
            );
        }
        StatusCode::OK => {
            println!(
                "Successfully send video request for {} to daemon on {:?} with response body: '{}'",
                yt_id,
                daemon_addr,
                String::from_utf8_lossy(&data_buf)
            );
        }
        _ => {
            eprintln!(
                "Video request for {} has been sent to daemon on {:?}, but daemon responded with unknown code {}! Response body: '{}'",
                yt_id,
                daemon_addr,
                response.status(),
                String::from_utf8_lossy(&data_buf)
            );
        }
    }
    // dropping daemon_connection closes the connection
}
