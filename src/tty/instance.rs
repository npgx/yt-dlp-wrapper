use crate::{BoxBodyBytes, BUFFER_SIZE, HEADER_INOUT_BIND};
use futures::{AsyncRead, AsyncWriteExt, Stream, StreamExt};
use http_body_util::BodyExt;
use pin_project::pin_project;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

#[pin_project]
struct PipeReaderToFrameStream {
    #[pin]
    reader: sluice::pipe::PipeReader,
    buffer: Vec<u8>,
}

impl Stream for PipeReaderToFrameStream {
    type Item = Result<hyper::body::Frame<hyper::body::Bytes>, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        let reader = this.reader;
        let buf = this.buffer;

        match reader.poll_read(cx, buf) {
            Poll::Ready(Ok(read_count)) => Poll::Ready(Some(Ok(hyper::body::Frame::data(
                hyper::body::Bytes::copy_from_slice(&buf[..read_count]),
            )))),
            Poll::Ready(Err(err)) => Poll::Ready(Some(Err(err))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl PipeReaderToFrameStream {
    fn from(reader: sluice::pipe::PipeReader, buf_capacity: usize) -> Self {
        Self {
            reader,
            buffer: vec![0; buf_capacity],
        }
    }
}

pub(crate) async fn handle_inout_bind_request(
    request: hyper::Request<hyper::body::Incoming>,
    addr: SocketAddr,
) -> Result<hyper::Response<BoxBodyBytes>, hyper::http::Error> {
    let (tcpin, mut tcpin_writer) = sluice::pipe::pipe();
    let (tcpout_reader, tcpout) = sluice::pipe::pipe();

    let tcpin_join = tokio::spawn(async move {
        let mut body_stream = request.into_data_stream();
        while let Some(chunk) = body_stream.next().await {
            let bytes = chunk.expect("Error receiving stdin bytes from tcp");
            tcpin_writer
                .write_all(&bytes)
                .await
                .expect("Error writing to stdin from tcp");
        }
    });

    *INOUT.lock().await = Some(InOut {
        from: Some(addr),
        stdin_join: Some(tcpin_join),
        stdin: tcpin,
        stdout_join: None,
        stdout: tcpout,
    });

    hyper::Response::builder()
        .header(HEADER_INOUT_BIND, "")
        .body(Box::new(BodyExt::map_err(
            http_body_util::StreamBody::new(PipeReaderToFrameStream::from(
                tcpout_reader,
                BUFFER_SIZE,
            )),
            |err| Box::new(err) as _,
        )))
}
