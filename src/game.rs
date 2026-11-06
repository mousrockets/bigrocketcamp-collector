//! Listens to Rocket League's Stats API on this PC and hands over each final scoreboard.
//!
//! Everything here blocks on the socket or sleeps: no polling while a match is played.

use crate::capture::{MatchCapture, StatsStream};
use serde_json::Value;
use std::net::{SocketAddr, TcpStream};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

/// The address the web app's bridge opens: `ws://localhost:49124/`.
const URL: &str = "ws://localhost:49124/";
const PORT: u16 = 49124;
const MAX_MESSAGE: usize = 1024 * 1024;
const FIRST_RETRY: Duration = Duration::from_secs(1);
const LAST_RETRY: Duration = Duration::from_secs(30);

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// Runs for the life of the process: connects, reads until the game closes, then waits and tries again.
/// `on_playing` tells when a match starts or stops being followed, so that an update can wait for its end.
pub fn listen(mut on_connection: impl FnMut(bool), mut on_match: impl FnMut(Value), mut on_playing: impl FnMut(bool)) {
    let mut retry = FIRST_RETRY;
    loop {
        if let Some(mut socket) = connect() {
            retry = FIRST_RETRY;
            on_connection(true);
            read(&mut socket, &mut on_match, &mut on_playing);
            on_connection(false);
        }
        sleep(retry);
        retry = (retry * 2).min(LAST_RETRY);
    }
}

/// Tries IPv4 first: on Windows a refused loopback connection can take seconds to report.
fn connect() -> Option<WebSocket<TcpStream>> {
    let addresses = [SocketAddr::from(([127, 0, 0, 1], PORT)), SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, PORT))];
    let config = WebSocketConfig::default().read_buffer_size(16 * 1024).max_message_size(Some(MAX_MESSAGE));
    addresses.iter().find_map(|address| {
        let stream = TcpStream::connect_timeout(address, Duration::from_secs(2)).ok()?;
        let _ = stream.set_nodelay(true);
        tungstenite::client::client_with_config(URL, stream, Some(config)).ok().map(|(socket, _)| socket)
    })
}

fn read(socket: &mut WebSocket<TcpStream>, on_match: &mut impl FnMut(Value), on_playing: &mut impl FnMut(bool)) {
    let mut stream = StatsStream::default();
    let mut capture = MatchCapture::default();
    let mut playing = false;
    loop {
        if capture.in_match() != playing {
            playing = !playing;
            on_playing(playing);
        }
        match socket.read() {
            Ok(Message::Text(text)) => match stream.push(text.as_str()) {
                Ok(messages) => {
                    for message in messages {
                        if let Some(payload) = capture.handle(&message, now()) {
                            on_match(payload);
                        }
                    }
                }
                // Like the browser: an unreadable message drops the match being followed, never a made-up result.
                Err(_) => capture.reset(),
            },
            Ok(Message::Binary(_)) => {
                stream.reset();
                capture.reset();
            }
            Ok(_) => {}
            Err(_) => {
                if playing {
                    on_playing(false);
                }
                return;
            }
        }
    }
}
