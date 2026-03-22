use serde::{Serialize, Deserialize};
use knot_core::utils::TimestampUtils;

pub mod daemon;

#[derive(Debug, Serialize, Deserialize)]
pub struct Message<TRequest, TResponse> {
    pub id: u32,
    pub timestamp: u64,
    pub kind: MessageKind<TRequest, TResponse>,
}


#[derive(Debug, Serialize, Deserialize)]
pub enum MessageKind<TRequest, TResponse>
{
    Request(TRequest),
    Response(TResponse)
}


impl<TRequest, TResponse> Message<TRequest, TResponse>
where
    TRequest:  Serialize,
    TResponse: Serialize,
{
    pub fn request(id: u32, payload: TRequest) -> Self {
        Self {
            id,
            timestamp: TimestampUtils::now_ms(),
            kind: MessageKind::Request(payload),
        }
    }

    pub fn response(id: u32, payload: TResponse) -> Self {
        Self {
            id,
            timestamp: TimestampUtils::now_ms(),
            kind: MessageKind::Response(payload),
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn into_request(self) -> Option<(u32, TRequest)> {
        match self.kind {
            MessageKind::Request(payload) => Some((self.id, payload)),
            _ => None,
        }
    }

    pub fn into_response(self) -> Option<(u32, TResponse)> {
        match self.kind {
            MessageKind::Response(payload) => Some((self.id, payload)),
            _ => None,
        }
    }

    pub fn is_request(&self) -> bool {
        matches!(self.kind, MessageKind::Request(_))
    }

    pub fn is_response(&self) -> bool {
        matches!(self.kind, MessageKind::Response(_))
    }
}