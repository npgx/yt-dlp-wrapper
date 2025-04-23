use crate::video;
use crate::{cli, lock};
use anyhow::anyhow;
use std::net::SocketAddr;
use std::time::Duration;

pub(crate) async fn run(args: cli::RequestArgs) -> Result<(), anyhow::Error> {
    let port = match (args.port, args.dangerously_skip_lock_checks) {
        (None, false) => {
            tokio::task::spawn_blocking(|| {
                lock::ensure_tty_running_and_read_port()
                    .expect("Failed to read daemon port from portfile! Is the daemon running?")
            })
            .await?
        }
        (None, true) => {
            panic!("ERROR: The lockfile check is set to be skipped, but no port has been specified!")
        }
        (Some(port), skip) => {
            if skip {
                println!("WARNING: Skipping lock check!");
            }
            println!("Using manually specified port {}", port);
            port
        }
    };

    let daemon_addr = format!("127.0.0.1:{}", port).parse::<SocketAddr>()?;
    let client = reqwest::Client::builder()
        .build()
        .expect("Failed to create http client!");

    println!("Creating video request...");
    let video_request = video::VideoRequest::from_yt_url(&args.yt_url, std::process::id())?;
    let yt_id = &video_request.youtube_id;

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
