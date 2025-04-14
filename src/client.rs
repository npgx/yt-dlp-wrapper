use crate::{video, BoxBodyBytes, HEADER_VIDEO_REQUEST};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub(crate) struct DaemonConnection {
    pub addr: SocketAddr,
    #[allow(dead_code)]
    pub connection_join: JoinHandle<()>,
    pub send_request: hyper::client::conn::http1::SendRequest<BoxBodyBytes>,
}

pub(crate) async fn connect_to_daemon(
    daemon_addr: SocketAddr,
) -> Result<DaemonConnection, std::io::Error> {
    let stream = TcpStream::connect(daemon_addr)
        .await
        .expect("Failed to connect to daemon!");

    let (sender, conn) = hyper::client::conn::http1::Builder::new()
        .handshake(TokioIo::new(stream))
        .await
        .expect("Failed to execute HTTP1 handshake");

    let conn_join = tokio::task::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("Connection error: {}", e);
        }
    });

    Ok(DaemonConnection {
        addr: daemon_addr,
        connection_join: conn_join,
        send_request: sender,
    })
}

pub(crate) async fn send_video_request_to_daemon(
    conn: &mut DaemonConnection,
    request: video::VideoRequest,
) -> hyper::Result<hyper::Response<hyper::body::Incoming>> {
    let req = hyper::Request::builder()
        .header(hyper::header::HOST, conn.addr.to_string())
        .header(HEADER_VIDEO_REQUEST, "")
        .body(request.into())
        .unwrap();

    conn.send_request.send_request(req).await
}
