use crate::video::VideoRequest;
use axum::response::Response;
use axum::routing::post;
use tokio::sync::mpsc::error::TrySendError;

#[derive(Clone)]
pub(crate) struct TtyAxumState {
    pub(crate) vreq_sender: tokio::sync::mpsc::Sender<VideoRequest>,
}

pub(crate) struct HttpMsgResponse {
    status_code: axum::http::StatusCode,
    msg: String,
}

impl HttpMsgResponse {
    pub(crate) fn new(status_code: axum::http::StatusCode, msg: String) -> Self {
        Self { status_code, msg }
    }
}

impl axum::response::IntoResponse for HttpMsgResponse {
    fn into_response(self) -> Response {
        (self.status_code, self.msg).into_response()
    }
}

pub(crate) mod post {
    use super::*;

    pub(crate) async fn video_request(
        axum::extract::State(state): axum::extract::State<TtyAxumState>,
        axum::Form(vreq): axum::Form<VideoRequest>,
    ) -> Result<(), HttpMsgResponse> {
        match state.vreq_sender.try_send(vreq) {
            Ok(_) => Ok(()),
            Err(error) => match error {
                TrySendError::Full(_) => Err(HttpMsgResponse::new(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    String::from("Cannot enqueue: Video request queue capacity exceeded!"),
                )),
                // shouldn't happen
                TrySendError::Closed(_) => Err(HttpMsgResponse::new(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    String::from("Cannot enqueue: Video request queue closed!"),
                )),
            },
        }
    }
}

pub(crate) async fn start_axum_app(
    vreq_sender: tokio::sync::mpsc::Sender<VideoRequest>,
    tcpl: tokio::net::TcpListener,
) {
    let app = axum::Router::new()
        .route("/video-request", post(post::video_request))
        .with_state(TtyAxumState { vreq_sender });

    axum::serve(tcpl, app).await.unwrap();
}
