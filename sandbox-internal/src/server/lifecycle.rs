//! Current lifecycle route bound to the original post-TLS peer value.
use super::{
    BodyExt, Bytes, CertificatePin, Full, Incoming, LifecycleReceiver, Limited, Method,
    ReceiveError, Request, Response, StatusCode, ValidationReceiver, json_response, response,
};
use crate::lifecycle::{CREATE_OBSERVATION_BODY_BYTES, CenterPeerRegistration, REQUEST_BODY_BYTES};
use sandproof_protocol::lifecycle::AcceptedLifecycleIntent;

#[derive(Clone, Copy)]
pub(super) enum Call {
    Record,
    CreateObservation,
    TerminalObservation,
    Confirm,
    Close,
    Validation,
}

pub(super) async fn respond<H: LifecycleReceiver + ValidationReceiver>(
    request: Request<Incoming>,
    peer: CertificatePin,
    registration: Option<&CenterPeerRegistration>,
    handler: &H,
    call: Call,
) -> Response<Full<Bytes>> {
    let Some(registration) = registration else {
        return response(StatusCode::NOT_FOUND, Bytes::new());
    };
    let Ok(context) = registration.context_for_verified_peer(peer) else {
        return response(StatusCode::UNAUTHORIZED, Bytes::new());
    };
    if request.uri().query().is_some() {
        return response(StatusCode::NOT_FOUND, Bytes::new());
    }
    if request.method() != Method::POST {
        return response(StatusCode::METHOD_NOT_ALLOWED, Bytes::new());
    }
    let limit = if matches!(call, Call::Validation) {
        crate::template::VALIDATION_REQUEST_BODY_BYTES
    } else if matches!(call, Call::Confirm | Call::Close) {
        sandproof_protocol::lifecycle::TERMINAL_RESULT_BODY_BYTES
    } else {
        REQUEST_BODY_BYTES
    };
    if request
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .map(hyper::header::HeaderValue::as_bytes)
        != Some(b"application/json")
        || request
            .headers()
            .contains_key(hyper::header::CONTENT_ENCODING)
    {
        return response(StatusCode::UNSUPPORTED_MEDIA_TYPE, Bytes::new());
    }
    if request
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|length| length > limit as u64)
    {
        return response(StatusCode::PAYLOAD_TOO_LARGE, Bytes::new());
    }
    let Ok(body) = Limited::new(request.into_body(), limit).collect().await else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if matches!(call, Call::Validation) {
        return super::validation_evidence(&body.to_bytes(), context, handler).await;
    }
    if matches!(call, Call::Confirm | Call::Close) {
        return terminal_command(&body.to_bytes(), context, handler, call).await;
    }
    let Ok(intent) = serde_json::from_slice::<AcceptedLifecycleIntent>(&body.to_bytes()) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if intent.validate(&context).is_err() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    if matches!(call, Call::CreateObservation) {
        return match handler.observe_create(intent.clone(), context).await {
            Ok(view) if view.matches_request(&intent) => {
                json_response(&view, CREATE_OBSERVATION_BODY_BYTES)
            }
            Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
            Ok(_) | Err(ReceiveError::Capacity | ReceiveError::Unavailable) => {
                response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
            }
        };
    }
    if matches!(call, Call::TerminalObservation) {
        return match handler.observe_terminal(intent.clone(), context).await {
            Ok(view) if view.matches_request(&intent) => json_response(&view, REQUEST_BODY_BYTES),
            Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
            Ok(_) | Err(ReceiveError::Capacity | ReceiveError::Unavailable) => {
                response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
            }
        };
    }
    match handler.record_lifecycle(intent.clone(), context).await {
        Ok(recorded) if recorded.matches(&intent) => json_response(&recorded, REQUEST_BODY_BYTES),
        Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
        Ok(_) | Err(ReceiveError::Capacity | ReceiveError::Unavailable) => {
            response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
        }
    }
}

async fn terminal_command<H: LifecycleReceiver>(
    bytes: &[u8],
    context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    handler: &H,
    call: Call,
) -> Response<Full<Bytes>> {
    use sandproof_protocol::lifecycle::{
        LifecycleWindowClose, TERMINAL_RESULT_BODY_BYTES, TerminalLifecycleObservation,
        TerminalStatus,
    };
    if matches!(call, Call::Confirm) {
        let Ok(result) = serde_json::from_slice::<TerminalLifecycleObservation>(bytes) else {
            return response(StatusCode::BAD_REQUEST, Bytes::new());
        };
        if result.request.validate(&context).is_err()
            || result.status != TerminalStatus::Cleaned
            || !result.matches_request(&result.request)
        {
            return response(StatusCode::CONFLICT, Bytes::new());
        }
        return match handler.confirm_terminal(result.clone(), context).await {
            Ok(ack) if ack.matches(&result) => json_response(&ack, TERMINAL_RESULT_BODY_BYTES),
            Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
            Ok(_) | Err(ReceiveError::Capacity | ReceiveError::Unavailable) => {
                response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
            }
        };
    }
    let Ok(request) = serde_json::from_slice::<LifecycleWindowClose>(bytes) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if request.terminal.request.validate(&context).is_err() || !request.is_terminal_result() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    match handler.close_window(request.clone(), context).await {
        Ok(ack) if ack.matches(&request) => json_response(&ack, TERMINAL_RESULT_BODY_BYTES),
        Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
        Ok(_) | Err(ReceiveError::Capacity | ReceiveError::Unavailable) => {
            response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
        }
    }
}
