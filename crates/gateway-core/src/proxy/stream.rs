use bytes::Bytes;
use futures::stream::{BoxStream, Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};

/// A streaming wrapper that copies every chunk into an accumulator while
/// passing it through unchanged. Used so the proxy can log full responses
/// and (later) feed cache + token accounting without buffering twice.
pub struct TeeStream {
    inner: BoxStream<'static, std::io::Result<Bytes>>,
    accumulator: Vec<u8>,
    max_capture: usize,
    truncated: bool,
}

impl TeeStream {
    pub fn new(inner: BoxStream<'static, std::io::Result<Bytes>>, max_capture: usize) -> Self {
        Self {
            inner,
            accumulator: Vec::new(),
            max_capture,
            truncated: false,
        }
    }

    pub fn captured(&self) -> &[u8] {
        &self.accumulator
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn into_captured(self) -> (Vec<u8>, bool) {
        (self.accumulator, self.truncated)
    }
}

impl Stream for TeeStream {
    type Item = std::io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                let remaining = self.max_capture.saturating_sub(self.accumulator.len());
                if remaining > 0 {
                    let take = remaining.min(chunk.len());
                    self.accumulator.extend_from_slice(&chunk[..take]);
                    if take < chunk.len() {
                        self.truncated = true;
                    }
                } else if !chunk.is_empty() {
                    self.truncated = true;
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            other => other,
        }
    }
}
