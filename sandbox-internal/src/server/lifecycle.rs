//! Current lifecycle route bound to the original post-TLS peer value.
use super::{
    BodyExt, Bytes, CertificatePin, Full, Incoming, LifecycleReceiver, Limited, Method,
    ReceiveError, Request, Response, StatusCode, json_response, response,
};
use crate::lifecycle::{CenterPeerRegistration, REQUEST_BODY_BYTES, CREATE_OBSERVATION_BODY_BYTES};
use sandproof_protocol::lifecycle::AcceptedLifecycleIntent;

#[derive(Clone, Copy)]
pub(super) enum Call {
    Record,
    CreateObservation,
    TerminalObservation,
}

pub(super) async fn respond<H: LifecycleReceiver>(
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
        .is_some_and(|length| length > REQUEST_BODY_BYTES as u64)
    {
        return response(StatusCode::PAYLOAD_TOO_LARGE, Bytes::new());
    }
    let Ok(body) = Limited::new(request.into_body(), REQUEST_BODY_BYTES)
        .collect()
        .await
    else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    let Ok(intent) = serde_json::from_slice::<AcceptedLifecycleIntent>(&body.to_bytes()) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if intent.validate(&context).is_err() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    if matches!(call, Call::CreateObservation) {
        return match handler.observe_create(intent.clone(), context).await {
            Ok(view) if view.matches_request(&intent) => json_response(&view, CREATE_OBSERVATION_BODY_BYTES),
            Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
            Ok(_) | Err(ReceiveError::Capacity | ReceiveError::Unavailable) => response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new()),
        };
    }
    if matches!(call, Call::TerminalObservation) {
        return match handler.observe_terminal(intent.clone(), context).await {
            Ok(view) if view.matches_request(&intent) => json_response(&view, REQUEST_BODY_BYTES),
            Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
            Ok(_) | Err(ReceiveError::Capacity | ReceiveError::Unavailable) => response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new()),
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
