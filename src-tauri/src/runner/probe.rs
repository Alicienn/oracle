//! Port probing and adoption of processes Oracle did not start.
//!
//! Two distinct jobs share this module because both answer questions about a TCP port.
//! `is_port_open` decides whether a project we launched has finished booting. `pid_on_port`
//! lets Oracle notice a dev server the user started from a terminal and show it as running
//! rather than pretending nothing is there.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::net::TcpStream;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);

/// True when something accepts a TCP connection on the loopback port.
///
/// A dev server binds its port only once it is genuinely ready to serve, which makes this a
/// far better readiness signal than "the process exists".
///
/// Both loopback stacks are tried, and either one answering is enough. Probing only IPv4
/// was wrong in a way that showed up as a healthy project reported as unresponsive: `next
/// dev -H localhost` resolves to `[::1]` on this platform and never binds `127.0.0.1`, so
/// the server was answering on a socket nothing was asking about.
pub async fn is_port_open(port: u16) -> bool {
    for ip in [IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)] {
        let addr = SocketAddr::new(ip, port);

        if matches!(
            tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await,
            Ok(Ok(_))
        ) {
            return true;
        }
    }

    false
}

/// Waits for the port to open, giving up after `timeout`.
///
/// Returns true if the port opened, false if the deadline passed. The caller decides what a
/// timeout means — for a project with a declared port it usually means the process is alive
/// but wedged, which is worth showing differently from "running".
pub async fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;

    while tokio::time::Instant::now() < deadline {
        if is_port_open(port).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    false
}

/// How many candidates `find_free_port` tries before giving up.
const SEARCH_WIDTH: u16 = 64;

/// True when a port cannot be bound because something else holds it.
///
/// Binding is the authoritative test, and `is_port_open` answers a different question —
/// whether something *accepts connections*. The two disagree in the case that matters here:
/// a server bound to one loopback stack only. Vite, for instance, listens on `[::1]` and
/// leaves `127.0.0.1` free, so a connect probe on IPv4 reports the port as available right
/// up until the child process fails with `EADDRINUSE`. Both stacks are therefore checked.
///
/// Only `AddrInUse` counts as taken. A machine with IPv6 disabled fails the second bind for
/// an unrelated reason, and reading that as a conflict would make every port look occupied.
pub fn port_is_taken(port: u16) -> bool {
    holds(IpAddr::V4(Ipv4Addr::LOCALHOST), port) || holds(IpAddr::V6(Ipv6Addr::LOCALHOST), port)
}

fn holds(ip: IpAddr, port: u16) -> bool {
    match std::net::TcpListener::bind((ip, port)) {
        Ok(_) => false,
        Err(err) => err.kind() == std::io::ErrorKind::AddrInUse,
    }
}

/// The first free port above `from`, or `None` if the whole search window is taken.
///
/// Walks upward rather than picking at random so the suggestion stays recognisable: a user
/// whose Next.js app normally answers on 3000 expects to be offered 3001, not 47318.
pub fn find_free_port(from: u16) -> Option<u16> {
    (1..=SEARCH_WIDTH)
        .filter_map(|step| from.checked_add(step))
        .find(|&port| !port_is_taken(port))
}

/// The PID listening on a loopback port, if any.
///
/// Shells out to `netstat` rather than taking a Windows API dependency: this runs at most
/// once per project when Oracle starts, so the process cost is irrelevant and the parsing is
/// easy to reason about.
#[cfg(windows)]
pub async fn pid_on_port(port: u16) -> Option<u32> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut command = std::process::Command::new("netstat");
    command.args(["-ano", "-p", "TCP"]);
    command.creation_flags(CREATE_NO_WINDOW);

    let output = tokio::process::Command::from(command).output().await.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);

    parse_netstat(&text, port)
}

#[cfg(not(windows))]
pub async fn pid_on_port(_port: u16) -> Option<u32> {
    // Only Windows is supported for now; on other platforms Oracle simply never adopts.
    None
}

/// Finds the PID of the first LISTENING row bound to `port`.
///
/// Rows look like:
///   TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       12345
fn parse_netstat(output: &str, port: u16) -> Option<u32> {
    let suffix = format!(":{port}");

    for line in output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        if !fields[3].eq_ignore_ascii_case("LISTENING") {
            continue;
        }
        // Match on the port only, not the address, so 0.0.0.0, 127.0.0.1 and [::] all count.
        if !fields[1].ends_with(&suffix) {
            continue;
        }
        if let Ok(pid) = fields[4].parse::<u32>() {
            return Some(pid);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1128
  TCP    127.0.0.1:3000         0.0.0.0:0              LISTENING       24680
  TCP    127.0.0.1:3000         127.0.0.1:51234        ESTABLISHED     24680
  TCP    [::]:5432              [::]:0                 LISTENING       9012
";

    #[test]
    fn finds_the_listening_pid_for_a_port() {
        assert_eq!(parse_netstat(SAMPLE, 3000), Some(24680));
    }

    #[test]
    fn ignores_established_connections() {
        // Port 51234 only ever appears as a foreign address, never as a listener.
        assert_eq!(parse_netstat(SAMPLE, 51234), None);
    }

    #[test]
    fn handles_ipv6_bracket_notation() {
        assert_eq!(parse_netstat(SAMPLE, 5432), Some(9012));
    }

    #[test]
    fn returns_none_for_an_unused_port() {
        assert_eq!(parse_netstat(SAMPLE, 9999), None);
    }

    #[test]
    fn does_not_match_a_port_that_is_only_a_suffix() {
        // Port 35 must not match the row for port 135.
        assert_eq!(parse_netstat(SAMPLE, 35), None);
    }

    #[tokio::test]
    async fn an_unused_port_reads_as_closed() {
        // Port 1 on loopback is not something a user process can bind without privileges.
        assert!(!is_port_open(1).await);
    }

    #[test]
    fn a_taken_port_is_skipped_for_the_next_one() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let taken = listener.local_addr().unwrap().port();

        // Searching from just below the bound port must step over it.
        let found = find_free_port(taken - 1).expect("some port above should be free");
        assert_ne!(found, taken);
        assert!(found > taken - 1);
    }

    #[test]
    fn a_suggested_port_can_actually_be_bound() {
        let port = find_free_port(3000).expect("a free port should exist above 3000");
        assert!(std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok());
    }

    #[test]
    fn a_bound_port_reads_as_taken() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        assert!(port_is_taken(port));
        drop(listener);
        assert!(!port_is_taken(port));
    }

    #[test]
    fn an_ipv6_only_listener_still_counts_as_taken() {
        // The Vite case: bound on [::1], nothing on 127.0.0.1. An IPv4-only check would
        // call this free and let a child process start straight into EADDRINUSE.
        let Ok(listener) = std::net::TcpListener::bind((Ipv6Addr::LOCALHOST, 0)) else {
            // No IPv6 on this machine; there is nothing to assert.
            return;
        };
        let port = listener.local_addr().unwrap().port();

        assert!(port_is_taken(port));
    }

    #[test]
    fn the_search_stops_rather_than_overflowing() {
        // There is no port above u16::MAX to suggest, and wrapping to 0 would be worse than
        // admitting defeat.
        assert_eq!(find_free_port(u16::MAX), None);
    }

    #[tokio::test]
    async fn a_bound_port_reads_as_open() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        assert!(is_port_open(port).await);
    }

    #[tokio::test]
    async fn a_server_on_ipv6_loopback_reads_as_open() {
        // What `next dev -H localhost` does. Probing IPv4 alone reported this as closed and
        // left a working project sitting at "Not responding" for a minute.
        let Ok(listener) = tokio::net::TcpListener::bind("[::1]:0").await else {
            return;
        };
        let port = listener.local_addr().unwrap().port();

        assert!(is_port_open(port).await);
    }
}
