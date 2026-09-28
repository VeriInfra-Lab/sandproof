//! Do not accept upstream success before all bounded input reaches Hyper.
//! This is not a receipt of a committed or atomic guest filesystem write.

use super::*;
use axum::body::Bytes;
use hyper::body::Frame;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
pub(super) struct UploadProgress {
    complete: Arc<AtomicBool>,
    over_limit: Arc<AtomicBool>,
    pub length: Option<u64>,
}

impl UploadProgress {
    pub fn complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    pub fn over_limit(&self) -> bool {
        self.over_limit.load(Ordering::Acquire)
    }
}

pub(super) struct UploadBody {
    inner: BodyStream<Limited<Body>>,
    progress: UploadProgress,
    transferred: u64,
}

impl UploadBody {
    pub fn new(body: Body, limit: usize) -> (Self, UploadProgress) {
        let length = body.size_hint().exact();
        let progress = UploadProgress {
            complete: Arc::new(AtomicBool::new(length == Some(0))),
            over_limit: Arc::new(AtomicBool::new(false)),
            length,
        };
        (
            Self {
                inner: BodyStream::new(Limited::new(body, limit)),
                progress: progress.clone(),
                transferred: 0,
            },
            progress,
        )
    }
}

impl hyper::body::Body for UploadBody {
    type Data = Bytes;
    type Error = Box<dyn Error + Send + Sync>;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let result = Pin::new(&mut self.inner).poll_frame(cx);
        match &result {
            Poll::Ready(None) => self.progress.complete.store(true, Ordering::Release),
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(bytes) = frame.data_ref() {
                    self.transferred += bytes.len() as u64;
                    // With Content-Length, Hyper need not poll another frame
                    // after consuming the declared length from this body.
                    if self.progress.length == Some(self.transferred) {
                        self.progress.complete.store(true, Ordering::Release);
                    }
                }
            }
            Poll::Ready(Some(Err(error))) => {
                self.progress.complete.store(false, Ordering::Release);
                if error.is::<http_body_util::LengthLimitError>() {
                    self.progress.over_limit.store(true, Ordering::Release);
                }
            }
            Poll::Pending => {}
        }
        result
    }
}
