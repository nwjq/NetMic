//! 控制面 JSON envelope（MVP：type + payload）。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 控制面消息类型常量。
pub const CONTROL_TYPE_HANDSHAKE_REQUEST: &str = "handshake_request";
pub const CONTROL_TYPE_HANDSHAKE_RESPONSE: &str = "handshake_response";
pub const CONTROL_TYPE_HEARTBEAT: &str = "heartbeat";
pub const CONTROL_TYPE_STATS: &str = "stats";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ControlEnvelope<T> {
    #[serde(rename = "type")]
    message_type: String,
    payload: T,
}

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("control json deserialize failed: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("control payload decode failed: {0}")]
    InvalidPayload(serde_json::Error),
}

/// 编码控制面消息为 JSON bytes。
pub fn encode_control_message<T: Serialize>(
    message_type: &str,
    payload: &T,
) -> Result<Vec<u8>, ControlError> {
    let envelope = ControlEnvelope {
        message_type: message_type.to_string(),
        payload,
    };
    Ok(serde_json::to_vec(&envelope)?)
}

/// 解析控制面消息，返回 (type, payload-json)。
pub fn decode_control_message(buf: &[u8]) -> Result<(String, serde_json::Value), ControlError> {
    let envelope: ControlEnvelope<serde_json::Value> = serde_json::from_slice(buf)?;
    Ok((envelope.message_type, envelope.payload))
}

/// 将 payload-json 解码为指定类型。
pub fn decode_control_payload<T: DeserializeOwned>(
    payload: serde_json::Value,
) -> Result<T, ControlError> {
    serde_json::from_value(payload).map_err(ControlError::InvalidPayload)
}

#[cfg(test)]
mod tests {
    use super::{decode_control_message, decode_control_payload, encode_control_message};
    use crate::protocol::{HandshakeRequest, HandshakeResponse, SessionParams};

    #[test]
    fn encode_and_decode_handshake_roundtrip() {
        let request = HandshakeRequest {
            session_id: "session-1".to_string(),
            client_name: "client".to_string(),
            requested: SessionParams::mvp_default(),
            token: None,
        };
        let bytes = encode_control_message("handshake_request", &request).expect("encode");
        let (msg_type, payload) = decode_control_message(&bytes).expect("decode");
        assert_eq!(msg_type, "handshake_request");
        let decoded: HandshakeRequest = decode_control_payload(payload).expect("payload decode");
        assert_eq!(decoded.session_id, request.session_id);
        assert_eq!(decoded.client_name, request.client_name);
    }

    #[test]
    fn decode_payload_error_bubbles() {
        let response = HandshakeResponse {
            session_id: "session-1".to_string(),
            accepted: true,
            reason: None,
            effective: SessionParams::mvp_default(),
            busy: false,
        };
        let bytes = encode_control_message("handshake_response", &response).expect("encode");
        let (_, payload) = decode_control_message(&bytes).expect("decode");
        let decoded: HandshakeResponse = decode_control_payload(payload).expect("payload decode");
        assert_eq!(decoded.session_id, response.session_id);
        assert!(decoded.accepted);
    }
}
