use crate::{cli, lock, net, signals, video};
use console::style;
use std::sync::Arc;

pub(crate) fn init(args: Arc<cli::TtyArgs>) -> (std::net::TcpListener, u16) {
    let mut instance_lock = if args.dangerously_skip_lock_checks {
        println!("{}", style("WARNING: Skipping lock check!").bold().red());
        None
    } else {
        Some(lock::InstanceLock::lock_or_panic())
    };

    if let Some(instance_lock) = instance_lock.as_mut() {
        instance_lock.with_guard_mut(|guard| {
            // temporarily write port 0 in lockfile
            lock::write_pid_port(guard, 0).expect("Failed to write PID/PORT to lockfile!");
        })
    };

    let tcpl = match args.port_override {
        None => std::net::TcpListener::bind("127.0.0.1:0").expect("Failed to bind TCP listener"),
        Some(port_override) => match std::net::TcpListener::bind(format!("127.0.0.1:{port_override}")) {
            Ok(tcpl) => tcpl,
            Err(err) => {
                panic!("Failed to bind TCP listener to explicitly-provided port {port_override}: {err}");
            }
        },
    };

    tcpl.set_nonblocking(true)
        .expect("Failed to set TCP listener to non-blocking mode");

    let port = tcpl
        .local_addr()
        .expect("Failed to retrieve local_addr from TcpListener")
        .port();

    if let Some(instance_lock) = instance_lock.as_mut() {
        instance_lock.with_guard_mut(|guard| {
            lock::write_pid_port(guard, port).expect("Failed to write TCP listener port to portfile!");
        })
    }

    if let Some(instance_lock) = instance_lock {
        // make sure the instance_lock lives for the whole program's lifetime
        Box::leak(Box::new(instance_lock));
    }

    (tcpl, port)
}

pub(crate) async fn run(args: Arc<cli::TtyArgs>) {
    let init_args = args.clone();
    let (tcpl, port) = tokio::task::spawn_blocking(move || init(init_args))
        .await
        .expect("Failed TTY initialization");

    // using a mpsc queue lets us asynchronously add to the queue,
    // but handle each request one at a time in the terminal
    let (vreq_send, vreq_receive) = tokio::sync::mpsc::channel(args.max_requests.clamp(1, 256) as usize);

    let axum_join = net::start_axum_app(vreq_send, tcpl);

    println!(
        "{} Listening on 127.0.0.1:{}",
        style("TTY instance is running!").green(),
        port
    );

    signals::spawn_ctrlc_listener().await;

    video::spawn_video_request_handler(vreq_receive, args).await;

    axum_join.await;
}
