use crate::{cli, lock, net, signals, video};
use console::style;
use std::time::Duration;

pub(crate) async fn run(args: cli::TtyArgs) {
    let mut lock = lock::get_lock().await.expect("Failed to create lock to lockfile");
    let mut guard = lock
        .try_write()
        .expect("Failed to acquire lock guard, is another daemon instance already running?");

    lock::write_pid(&mut guard)
        .await
        .expect("Failed to write PID to lockfile!");

    let tcpl = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind TCP listener");
    let port = tcpl.local_addr().unwrap().port();

    lock::write_port(&mut guard, port)
        .await
        .expect("Failed to write TCP listener port to portfile!");

    // using a mpsc queue lets us asynchronously add to the queue,
    // but handle each request one at a time in the terminal
    let (tx, mut rx) = tokio::sync::mpsc::channel(args.max_requests.clamp(1, 128));

    let axum_join = net::start_axum_app(tx, tcpl);

    println!(
        "{} Listening on 127.0.0.1:{}",
        style("Daemon is running!").green(),
        port
    );

    signals::spawn_ctrlc_listener().await;

    tokio::spawn(async move {
        let mut acoustid_client = reqwest::Client::builder()
            .connector_layer(
                tower::ServiceBuilder::new()
                    .layer(tower::buffer::BufferLayer::new(16))
                    .layer(tower::timeout::TimeoutLayer::new(Duration::from_secs(2)))
                    .layer(tower::limit::RateLimitLayer::new(3, Duration::from_secs(1))),
            )
            .https_only(true)
            .build()
            .expect("Could not initialize acoust_id reqwest client.");

        while let Some(vreq) = rx.recv().await {
            let result = video::handle_video_request(vreq, &args, &mut acoustid_client).await;

            match result {
                Ok(true) => {}
                Ok(false) => {}
                Err(error) => {
                    eprintln!(
                        "{}\n{error}",
                        style("Failed to handle video request!").for_stderr().red()
                    )
                }
            }
        }
    });

    axum_join.await;
}
