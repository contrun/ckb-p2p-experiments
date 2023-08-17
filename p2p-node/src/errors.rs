use thiserror::Error;

use crate::network::CKBNetworkTypeError;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("{0:?}")]
    CKBNetworkTypeError(#[from] CKBNetworkTypeError),
}
