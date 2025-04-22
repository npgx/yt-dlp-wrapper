use crate::{cli, lock, net, signals, video};
use console::style;
use tokio::net::TcpListener;

pub(crate) async fn init() -> (TcpListener, u16) {
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

    (tcpl, port)
}

pub(crate) async fn run(args: cli::TtyArgs) {
    let (tcpl, port) = init().await;

    // using a mpsc queue lets us asynchronously add to the queue,
    // but handle each request one at a time in the terminal
    let (vreq_send, vreq_receive) = tokio::sync::mpsc::channel(args.max_requests.clamp(1, 128));

    let axum_join = net::start_axum_app(vreq_send, tcpl);

    println!(
        "{} Listening on 127.0.0.1:{}",
        style("Daemon is running!").green(),
        port
    );

    signals::spawn_ctrlc_listener().await;

    video::spawn_video_request_handler(vreq_receive, args).await;

    axum_join.await;
}
