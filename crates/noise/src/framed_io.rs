//! Bridge a `Stream<Item = Result<B, E>>` + `Sink<&[u8]>` into
//! `AsyncRead + AsyncWrite`.
//!
//! `ts_control_noise` has an identical helper (`FramedIo`) but does not export
//! it, so this is a faithful copy of that BSD-3 code (itself a copy of the
//! `tokio_util` `StreamReader`/`SinkWriter` impls fused into one type to avoid
//! the mutexes a `split` + `join` would impose). Used to wrap the encrypted
//! `Framed<T, BiCodec>` transport as a byte stream the HTTP/2 server can drive.

use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use bytes::Buf;
use futures_util::{Sink, Stream};
use tokio::io::{AsyncBufRead, AsyncRead, AsyncWrite, ReadBuf};

pin_project_lite::pin_project! {
    /// Turns an inner `Stream<B>` + `Sink<&[u8]>` into `AsyncRead + AsyncWrite`.
    pub struct FramedIo<T, B> {
        #[pin]
        inner: T,
        chunk: Option<B>,
    }
}

impl<T, B> FramedIo<T, B> {
    /// Construct a new `FramedIo` around the inner `Stream` + `Sink`.
    pub const fn new(inner: T) -> Self {
        Self { inner, chunk: None }
    }
}

impl<T, B> FramedIo<T, B>
where
    B: Buf,
{
    /// Do we have a chunk and is it non-empty?
    fn has_chunk(&self) -> bool {
        if let Some(ref chunk) = self.chunk {
            chunk.remaining() > 0
        } else {
            false
        }
    }
}

impl<T, B, E> AsyncRead for FramedIo<T, B>
where
    T: Stream<Item = Result<B, E>>,
    B: Buf,
    E: Into<std::io::Error>,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }

        let inner_buf = match self.as_mut().poll_fill_buf(cx) {
            Poll::Ready(Ok(buf)) => buf,
            Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            Poll::Pending => return Poll::Pending,
        };
        let len = std::cmp::min(inner_buf.len(), buf.remaining());
        buf.put_slice(&inner_buf[..len]);

        self.consume(len);
        Poll::Ready(Ok(()))
    }
}

impl<T, B, E> AsyncBufRead for FramedIo<T, B>
where
    T: Stream<Item = Result<B, E>>,
    B: Buf,
    E: Into<std::io::Error>,
{
    fn poll_fill_buf(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<&[u8]>> {
        loop {
            if self.as_mut().has_chunk() {
                // This unwrap is very sad, but it can't be avoided.
                let buf = self.project().chunk.as_ref().unwrap().chunk();
                return Poll::Ready(Ok(buf));
            } else {
                match self.as_mut().project().inner.poll_next(cx) {
                    Poll::Ready(Some(Ok(chunk))) => {
                        // Go around the loop in case the chunk is empty.
                        *self.as_mut().project().chunk = Some(chunk);
                    }
                    Poll::Ready(Some(Err(err))) => return Poll::Ready(Err(err.into())),
                    Poll::Ready(None) => return Poll::Ready(Ok(&[])),
                    Poll::Pending => return Poll::Pending,
                }
            }
        }
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        if amt > 0 {
            self.project()
                .chunk
                .as_mut()
                .expect("No chunk present")
                .advance(amt);
        }
    }
}

impl<T, B, E> AsyncWrite for FramedIo<T, B>
where
    T: for<'a> Sink<&'a [u8], Error = E>,
    E: Into<std::io::Error>,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut this = self.project();

        ready!(this.inner.as_mut().poll_ready(cx).map_err(Into::into))?;
        match this.inner.as_mut().start_send(buf) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(e) => Poll::Ready(Err(e.into())),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.project().inner.poll_flush(cx).map_err(Into::into)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.project().inner.poll_close(cx).map_err(Into::into)
    }
}
