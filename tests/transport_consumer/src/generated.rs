// GENERATED native Rust types + codec — do not edit.
#![allow(dead_code)]
use crate::cbor::{Cbor, DecodeError};
pub use gwz_transport::protocol::AuthMethod;
pub use gwz_transport::protocol::AuthPolicy;
pub use gwz_transport::protocol::Barrier;
pub use gwz_transport::protocol::Bind;
pub use gwz_transport::protocol::Bound;
pub use gwz_transport::protocol::Cancel;
pub use gwz_transport::protocol::CheckIdentity;
pub use gwz_transport::protocol::Close;
pub use gwz_transport::protocol::Closed;
pub use gwz_transport::protocol::Data;
pub use gwz_transport::protocol::Deadlines;
pub use gwz_transport::protocol::Destination;
pub use gwz_transport::protocol::Disposition;
pub use gwz_transport::protocol::Effect;
pub use gwz_transport::protocol::EndWrite;
pub use gwz_transport::protocol::EndpointRole;
pub use gwz_transport::protocol::Envelope;
pub use gwz_transport::protocol::ErrorCode;
pub use gwz_transport::protocol::Facts;
pub use gwz_transport::protocol::Failure;
pub use gwz_transport::protocol::GitService;
pub use gwz_transport::protocol::Identity;
pub use gwz_transport::protocol::IdentityChecked;
pub use gwz_transport::protocol::IdentityMode;
pub use gwz_transport::protocol::Limits;
pub use gwz_transport::protocol::MessageKind;
pub use gwz_transport::protocol::Open;
pub use gwz_transport::protocol::Opened;
pub use gwz_transport::protocol::Scheme;
pub use gwz_transport::protocol::Window;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GwzTransportDelivery {
    pub message: Envelope,
}
impl GwzTransportDelivery {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![(1, self.message.to_cbor())])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            message: Envelope::from_cbor(c.try_get(1)?)?,
        })
    }
}
