use std::error::Error;

use futures::prelude::*;
use url::Url;

mod tcp;
mod tls;
mod tor;

pub use tcp::TcpTransport;
pub use tls::TlsTransport;
pub use tor::{TorController, TorTransport};

pub trait Transport {
    type Acceptor;
    type Connector;

    type Error: Error;

    type Listener: Future<Output = Result<Self::Acceptor, Self::Error>>;
    type Dial: Future<Output = Result<Self::Connector, Self::Error>>;

    fn listen_on(self, url: Url) -> Result<Self::Listener, TransportError<Self::Error>>
    where
        Self: Sized;

    fn dial(self, url: Url) -> Result<Self::Dial, TransportError<Self::Error>>
    where
        Self: Sized;
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError<TErr> {
    #[error("Address not supported: {0}")]
    AddrNotSupported(Url),

    #[error("Transport IO Error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Socks5 Error: {0}")]
    Socks5Error(#[from] fast_socks5::SocksError),

    #[error("{0}")]
    Other(TErr),
}
