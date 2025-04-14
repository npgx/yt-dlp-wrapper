use crate::{video, BoxBodyBytes, HEADER_VIDEO_REQUEST};
use http_body_util::BodyExt;
use hyper::service::service_fn;
use hyper::StatusCode;
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

pub(crate) async fn handle_tcp_connections(
    tcp_listener: TcpListener,
    tx: Sender<video::VideoRequest>,
) {
    let mut connections: Vec<JoinHandle<()>> = Vec::new();

    loop {
        let future = tcp_listener.accept();
        // TODO: add timeout that *won't* close the daemon in the middle of it being active
        //let future = tokio::time::timeout(Duration::from_secs(600), future);
        match future.await {
            Err(err) => {
                eprintln!("TCP connection error: {}", err);
            }
            Ok((stream, addr)) => {
                connections.push(tokio::spawn(handle_tcp_stream(stream, addr, tx.clone())));
            }
        }
    }
}

pub(crate) async fn handle_tcp_stream(
    stream: TcpStream,
    addr: SocketAddr,
    tx: Sender<video::VideoRequest>,
) {
    let conn = hyper::server::conn::http1::Builder::new().serve_connection(
        TokioIo::new(stream),
        service_fn(move |req| {
            let addr = addr.clone();
            let tx = tx.clone();

            route_http_request(req, addr, tx)
        }),
    );

    if let Err(e) = conn.await {
        eprintln!("Error serving connection: {:?}", e);
    }
}

pub(crate) async fn route_http_request(
    request: hyper::Request<hyper::body::Incoming>,
    addr: SocketAddr,
    tx: Sender<video::VideoRequest>,
) -> Result<hyper::Response<BoxBodyBytes>, hyper::http::Error> {
    if request.headers().contains_key(HEADER_VIDEO_REQUEST) {
        video::handle_video_http_request(request, addr, tx).await
    }
    /*else if request.headers().contains_key(HEADER_INOUT_BIND) {
        instance::handle_inout_bind_request(request, addr).await
    }*/
    else {
        hyper::Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Box::new(
                http_body_util::Empty::default().map_err(|err| Box::new(err) as _),
            ))
    }
}
