use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use tokio::net::TcpStream;

use crate::{Metadata, MetadataSource, ServiceError};

#[cfg(test)]
mod tests;

/// Explicit per-exchange budgets. The two MMDS requests share one deadline.
#[derive(Clone, Copy)]
pub struct MetadataLimits {
    pub body_bytes: usize,
    /// Hyper's receive buffer has a minimum of 8192 bytes. Kernel/socket and
    /// library allocations remain external to the retained payload budget.
    pub receive_bytes: usize,
    pub timeout: Duration,
}

pub struct MmdsClient {
    address: SocketAddr,
    limits: MetadataLimits,
}

impl MmdsClient {
    /// Fixed link-local Firecracker MMDS endpoint. No request, environment
    /// variable, DNS resolver, HTTP redirect or proxy selects another target.
    /// # Errors
    /// Refuses zero/invalid or unrepresentable budgets before opening sockets.
    pub fn firecracker(limits: MetadataLimits) -> Result<Self, ServiceError> {
        if limits.body_bytes == 0
            || limits.receive_bytes < 8192
            || limits.timeout.is_zero()
            || limits.receive_bytes > isize::MAX as usize
            || limits.body_bytes > isize::MAX as usize
            || tokio::time::Instant::now()
                .checked_add(limits.timeout)
                .is_none()
        {
            return Err(ServiceError::InvalidArgument);
        }
        Ok(Self {
            address: (Ipv4Addr::new(169, 254, 169, 254), 80).into(),
            limits,
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        token: Option<&[u8]>,
    ) -> Result<Vec<u8>, ServiceError> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", self.address.to_string())
            .header("connection", "close");
        request = if let Some(token) = token {
            request
                .header("x-metadata-token", token)
                .header("accept", "application/json")
        } else {
            request.header("x-metadata-token-ttl-seconds", "60")
        };
        let request = request
            .body(Empty::<Bytes>::new())
            .map_err(|_| ServiceError::Unavailable)?;
        // Validate all header bytes before creating a connection. A malformed
        // MMDS token must not even open the follow-up metadata socket.
        let connection = TcpStream::connect(self.address)
            .await
            .map_err(|_| ServiceError::Unavailable)?;
        let (mut sender, driver) = hyper::client::conn::http1::Builder::new()
            .max_headers(32)
            .max_buf_size(self.limits.receive_bytes)
            .handshake(TokioIo::new(connection))
            .await
            .map_err(|_| ServiceError::Unavailable)?;
        let exchange = async move {
            let response = sender
                .send_request(request)
                .await
                .map_err(|_| ServiceError::Unavailable)?;
            if response.status() != StatusCode::OK {
                return Err(ServiceError::Unavailable);
            }
            let mut body = response.into_body();
            let mut bytes = Vec::new();
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|_| ServiceError::Unavailable)?;
                if let Some(data) = frame.data_ref() {
                    if data.len() > self.limits.body_bytes - bytes.len() {
                        return Err(ServiceError::Unavailable);
                    }
                    bytes.extend_from_slice(data);
                }
            }
            Ok(bytes)
        };
        // Drive both futures in this call, without a detached connection task.
        // On timeout/cancellation/error, dropping them closes the owned socket.
        let (body, ()) = tokio::try_join!(exchange, async {
            driver.await.map_err(|_| ServiceError::Unavailable)
        })?;
        Ok(body)
    }
}

impl MetadataSource for MmdsClient {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        tokio::time::timeout(self.limits.timeout, async {
            let token = self.request(Method::PUT, "/latest/api/token", None).await?;
            if token.is_empty() {
                return Err(ServiceError::Unavailable);
            }
            let body = self.request(Method::GET, "/", Some(&token)).await?;
            if body.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
                return Err(ServiceError::Unavailable);
            }
            let fields: Fields =
                serde_json::from_slice(&body).map_err(|_| ServiceError::Unavailable)?;
            if fields.instance_id.is_empty()
                || fields.instance_id.contains('\0')
                || fields.template_id.is_empty()
                || fields.template_id.contains('\0')
            {
                return Err(ServiceError::Unavailable);
            }
            Ok(Metadata {
                instance_id: fields.instance_id,
                template_id: fields.template_id,
                access_token_sha512: decode_hash(fields.access_token_hash.as_bytes())?,
            })
        })
        .await
        .map_err(|_| ServiceError::Unavailable)?
    }
}

#[derive(Deserialize)]
struct Fields {
    #[serde(rename = "instanceID")]
    instance_id: String,
    #[serde(rename = "envID")]
    template_id: String,
    #[serde(rename = "accessTokenHash")]
    access_token_hash: String,
}

fn decode_hash(encoded: &[u8]) -> Result<[u8; 64], ServiceError> {
    if encoded.len() != 128 {
        return Err(ServiceError::Unavailable);
    }
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(ServiceError::Unavailable),
    };
    let mut decoded = [0; 64];
    for (out, pair) in decoded.iter_mut().zip(encoded.chunks_exact(2)) {
        *out = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Ok(decoded)
}
