use futures::future::BoxFuture;
use futures::prelude::*;
use libp2p::core::upgrade::{InboundUpgrade, OutboundUpgrade};
use libp2p::core::UpgradeInfo;
use log::debug;
use std::{
    io, iter,
    pin::Pin,
    task::{Context, Poll},
};

// The unstable never type is more appropirated, but it is unstable.
type Error = ();

/// [`PlainText`] is an insecure connection handshake for testing purposes only.
#[derive(Clone)]
pub struct PlainText;

impl PlainText {
    pub fn new() -> Self {
        Self {}
    }
}

impl UpgradeInfo for PlainText {
    type Info = &'static str;
    type InfoIter = iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once("/tentacle/0.0.1")
    }
}

impl<C> InboundUpgrade<C> for PlainText
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = Output<C>;
    type Error = Error;
    type Future = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, _: Self::Info) -> Self::Future {
        Box::pin(self.handshake(socket))
    }
}

impl<C> OutboundUpgrade<C> for PlainText
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = Output<C>;
    type Error = Error;
    type Future = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: C, _: Self::Info) -> Self::Future {
        Box::pin(self.handshake(socket))
    }
}

impl PlainText {
    async fn handshake<T>(self, socket: T) -> Result<Output<T>, Error>
    where
        T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        debug!("Starting plaintext handshake.");
        Ok(Output { socket })
    }
}

/// Output of the plaintext protocol.
pub struct Output<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// The plaintext stream.
    pub socket: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for Output<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, io::Error>> {
        AsyncRead::poll_read(Pin::new(&mut self.socket), cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for Output<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        AsyncWrite::poll_write(Pin::new(&mut self.socket), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.socket), cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        AsyncWrite::poll_close(Pin::new(&mut self.socket), cx)
    }
}
