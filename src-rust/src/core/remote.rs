//! HTTP health checks for projects deployed elsewhere.
//!
//! Deliberately the dumbest thing that works: one request per remote project, recording the
//! status code and how long it took. That is enough to answer "is it up?" without asking the
//! user to install anything on their server, and it works the same whether the app sits
//! behind Docker, systemd, or a reverse proxy.

use crate::core::config::model::{HttpMethod, RemoteCheck, RemoteTarget};
use serde::Serialize;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum RemoteStatus {
    /// Answered with an acceptable status code.
    #[serde(rename_all = "camelCase")]
    Up { ms: u64, code: u16 },
    /// Answered, but not with a status the project considers healthy.
    #[serde(rename_all = "camelCase")]
    Degraded { ms: u64, code: u16 },
    /// Did not answer at all.
    #[serde(rename_all = "camelCase")]
    Down { reason: String },
    /// Never checked yet.
    Unchecked,
}

impl RemoteStatus {
    pub fn latency_ms(&self) -> Option<u64> {
        match self {
            Self::Up { ms, .. } | Self::Degraded { ms, .. } => Some(*ms),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteReport {
    pub project_id: String,
    pub status: RemoteStatus,
    pub checked_at: i64,
}

/// Builds the shared client.
///
/// Redirects are not followed: a deployment that has started redirecting to a login page or
/// a parked domain is exactly the kind of breakage worth seeing, and silently following it
/// would report the wrong thing as healthy.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Oracle/", env!("CARGO_PKG_VERSION")))
        .build()
        // A client that cannot be configured is a programming error, not a runtime
        // condition; the default client is a correct fallback.
        .unwrap_or_default()
}

/// Runs one check and classifies the result.
pub async fn check(client: &reqwest::Client, target: &RemoteTarget) -> RemoteStatus {
    let RemoteCheck::Http {
        method,
        expect_status,
    } = &target.check;

    let request = match method {
        HttpMethod::Get => client.get(&target.url),
        HttpMethod::Head => client.head(&target.url),
    };

    let started = Instant::now();

    match request.send().await {
        Ok(response) => {
            let ms = started.elapsed().as_millis() as u64;
            let code = response.status().as_u16();

            if is_healthy(code, expect_status) {
                RemoteStatus::Up { ms, code }
            } else {
                RemoteStatus::Degraded { ms, code }
            }
        }
        Err(err) => RemoteStatus::Down {
            reason: describe(&err),
        },
    }
}

/// An empty expectation list means "any 2xx or 3xx".
fn is_healthy(code: u16, expected: &[u16]) -> bool {
    if expected.is_empty() {
        (200..400).contains(&code)
    } else {
        expected.contains(&code)
    }
}

/// Turns a reqwest error into something worth showing in a tooltip.
///
/// The default `Display` for these is long and full of internals; the user only needs to
/// know which kind of failure it was.
fn describe(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        format!("No response within {}s", TIMEOUT.as_secs())
    } else if err.is_connect() {
        "Connection refused".to_string()
    } else if err.is_request() {
        "Invalid request".to_string()
    } else if err.is_body() || err.is_decode() {
        "Malformed response".to_string()
    } else {
        "Unreachable".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::model::RemoteCheck;

    fn target(url: &str, expect: Vec<u16>) -> RemoteTarget {
        RemoteTarget {
            url: url.to_string(),
            check: RemoteCheck::Http {
                method: HttpMethod::Get,
                expect_status: expect,
            },
            interval_secs: 30,
        }
    }

    #[test]
    fn any_success_or_redirect_counts_as_healthy_by_default() {
        assert!(is_healthy(200, &[]));
        assert!(is_healthy(204, &[]));
        assert!(is_healthy(301, &[]));
        assert!(is_healthy(399, &[]));
    }

    #[test]
    fn errors_are_not_healthy_by_default() {
        assert!(!is_healthy(404, &[]));
        assert!(!is_healthy(500, &[]));
        assert!(!is_healthy(502, &[]));
    }

    #[test]
    fn an_explicit_expectation_overrides_the_default() {
        // A project whose health endpoint legitimately answers 418.
        assert!(is_healthy(418, &[418]));
        // And which should therefore treat a plain 200 as wrong.
        assert!(!is_healthy(200, &[418]));
    }

    #[tokio::test]
    async fn a_refused_connection_reports_down() {
        // Port 1 on loopback has nothing listening.
        let status = check(&client(), &target("http://127.0.0.1:1/", vec![])).await;

        match status {
            RemoteStatus::Down { reason } => assert!(!reason.is_empty()),
            other => panic!("expected Down, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_malformed_url_reports_down_rather_than_panicking() {
        let status = check(&client(), &target("not-a-url", vec![])).await;
        assert!(matches!(status, RemoteStatus::Down { .. }));
    }

    /// Serves exactly one canned HTTP response and returns the port it listens on.
    ///
    /// The request must be drained before replying: closing the socket while the client is
    /// still writing its request headers produces a connection reset, which reqwest reports
    /// as a send failure rather than as the response the test means to assert on.
    async fn one_shot_server(response: &'static [u8]) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };

            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let mut seen: Vec<u8> = Vec::new();
            let mut buffer = [0u8; 1024];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                match socket.read(&mut buffer).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => seen.extend_from_slice(&buffer[..n]),
                }
            }

            let _ = socket.write_all(response).await;
            let _ = socket.flush().await;
            // Let the client finish reading before the socket drops.
            tokio::time::sleep(Duration::from_millis(50)).await;
        });

        port
    }

    #[tokio::test]
    async fn a_live_server_reports_up_with_a_latency() {
        let port = one_shot_server(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;

        let status = check(&client(), &target(&format!("http://127.0.0.1:{port}/"), vec![])).await;

        match status {
            RemoteStatus::Up { code, .. } => assert_eq!(code, 200),
            other => panic!("expected Up, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_server_error_reports_degraded_not_down() {
        let port =
            one_shot_server(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n").await;

        let status = check(&client(), &target(&format!("http://127.0.0.1:{port}/"), vec![])).await;

        // The distinction matters: the server is reachable, the app behind it is not well.
        match status {
            RemoteStatus::Degraded { code, .. } => assert_eq!(code, 503),
            other => panic!("expected Degraded, got {other:?}"),
        }
    }
}
