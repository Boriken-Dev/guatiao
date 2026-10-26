// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! WebSocket (RFC 6455) over TCP, at the trusted level: what a browser
//! can open. Each frame's body is one binary message, with no length in
//! front; the listener admits peers by address range as
//! [`TcpListen`](super::tcp::TcpListen) does.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Mutex, TryLockError};

use tungstenite::protocol::{Role, WebSocketConfig};
use tungstenite::{Message, WebSocket};

use super::tcp::{Accepted, CONNECT_TIMEOUT, Range, TcpListen};
use super::{Connect, Connection, FrameRead, FrameWrite};

/// The socket both halves use. Every write goes out whole under one lock:
/// tungstenite hands a write only whole frames, so the reader's pongs and
/// close replies never land inside a frame the writer is sending.
#[derive(Clone)]
struct Socket {
    stream: Arc<TcpStream>,
    writing: Arc<Mutex<()>>,
    /// The reader's: a write that finds the writer busy answers
    /// `WouldBlock` having written nothing, and tungstenite sends the
    /// reply on a later read, so the reader never waits behind a writer
    /// that waits for the peer to read.
    yields: bool,
}

impl Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self.stream).read(buf)
    }
}

impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let taken = if self.yields {
            self.writing.try_lock()
        } else {
            self.writing.lock().map_err(TryLockError::from)
        };
        let _whole = match taken {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(io::ErrorKind::WouldBlock.into()),
        };
        (&*self.stream).write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn config() -> WebSocketConfig {
    // Each frame goes out as soon as it is written; the link batches.
    WebSocketConfig::default().write_buffer_size(0)
}

fn io_error(e: tungstenite::Error) -> io::Error {
    match e {
        tungstenite::Error::Io(e) => e,
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
            io::ErrorKind::BrokenPipe.into()
        }
        tungstenite::Error::Capacity(why) => {
            io::Error::new(io::ErrorKind::InvalidData, why.to_string())
        }
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

/// What the reading half holds: the server's end upgrades on its first
/// read, when the client's HTTP request is due.
enum Reading {
    Upgrade,
    Open(Box<WebSocket<Socket>>),
    Ended,
}

struct WsReader {
    reading: Reading,
    socket: Socket,
    open: Arc<AtomicBool>,
}

impl WsReader {
    fn ws(&mut self) -> io::Result<Option<&mut WebSocket<Socket>>> {
        if matches!(self.reading, Reading::Upgrade) {
            let ws = tungstenite::accept_with_config(self.socket.clone(), Some(config())).map_err(
                |e| match e {
                    tungstenite::HandshakeError::Failure(e) => io_error(e),
                    tungstenite::HandshakeError::Interrupted(_) => io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the client's WebSocket upgrade did not arrive in time",
                    ),
                },
            )?;
            let mut ws = Box::new(ws);
            ws.get_mut().yields = true;
            self.reading = Reading::Open(ws);
            self.open.store(true, Ordering::Release);
        }
        Ok(match &mut self.reading {
            Reading::Open(ws) => Some(&mut **ws),
            _ => None,
        })
    }
}

impl FrameRead for WsReader {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        let Some(ws) = self.ws()? else {
            return Ok(false);
        };
        ws.set_config(|c| {
            c.max_message_size = Some(max);
            c.max_frame_size = Some(max);
        });
        loop {
            match ws.read() {
                Ok(Message::Binary(body)) => {
                    buf.clear();
                    buf.extend_from_slice(&body);
                    return Ok(true);
                }
                Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
                Ok(Message::Text(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "a text message: frames are binary",
                    ));
                }
                Ok(Message::Close(_)) => {
                    // The reply to the close goes out now, not on a next read.
                    let _ = ws.flush();
                    self.reading = Reading::Ended;
                    return Ok(false);
                }
                // A dropped connection ends the link as a stream's end of
                // file does; tungstenite does not say where it fell.
                Err(
                    tungstenite::Error::ConnectionClosed
                    | tungstenite::Error::AlreadyClosed
                    | tungstenite::Error::Protocol(
                        tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    ),
                ) => {
                    self.reading = Reading::Ended;
                    return Ok(false);
                }
                Err(tungstenite::Error::Capacity(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("a message over the {max} bytes allowed"),
                    ));
                }
                Err(e) => return Err(io_error(e)),
            }
        }
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.socket.stream.set_read_timeout(timeout)
    }
}

struct WsWriter {
    ws: WebSocket<Socket>,
    socket: Socket,
    open: Arc<AtomicBool>,
}

impl WsWriter {
    fn send(&mut self, body: tungstenite::Bytes) -> io::Result<()> {
        if !self.open.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "the WebSocket upgrade has not happened: a server's end reads first",
            ));
        }
        self.ws.write(Message::Binary(body)).map_err(io_error)
    }
}

/// The body of a frame the link built: the frame without its length.
fn body(frame: &[u8]) -> io::Result<&[u8]> {
    frame
        .get(4..)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a frame without its length"))
}

impl FrameWrite for WsWriter {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.send(tungstenite::Bytes::copy_from_slice(body(frame)?))?;
        self.ws.flush().map_err(io_error)
    }

    fn write_frame_parts(&mut self, head: &[u8], tail: &[u8]) -> io::Result<()> {
        let head = body(head)?;
        let mut joined = Vec::with_capacity(head.len() + tail.len());
        joined.extend_from_slice(head);
        joined.extend_from_slice(tail);
        self.send(joined.into())?;
        self.ws.flush().map_err(io_error)
    }

    fn write_frames(&mut self, frames: &[&[u8]]) -> io::Result<()> {
        for frame in frames {
            self.send(tungstenite::Bytes::copy_from_slice(body(frame)?))?;
        }
        self.ws.flush().map_err(io_error)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.socket.stream.set_write_timeout(timeout)
    }

    fn shutdown(&mut self) {
        if self.open.load(Ordering::Acquire) {
            let _ = self.ws.close(None);
            let _ = self.ws.flush();
        }
        let _ = self.socket.stream.shutdown(Shutdown::Both);
    }
}

fn shared(stream: TcpStream) -> io::Result<Socket> {
    stream.set_nodelay(true)?;
    Ok(Socket {
        stream: Arc::new(stream),
        writing: Arc::new(Mutex::new(())),
        yields: false,
    })
}

fn connection(socket: Socket, peer: String, reading: Reading, role: Role) -> Connection {
    let open = Arc::new(AtomicBool::new(matches!(reading, Reading::Open(_))));
    let closing = Arc::clone(&socket.stream);
    Connection {
        reader: Box::new(WsReader {
            reading,
            socket: socket.clone(),
            open: Arc::clone(&open),
        }),
        writer: Box::new(WsWriter {
            ws: WebSocket::from_raw_socket(socket.clone(), role, Some(config())),
            socket,
            open,
        }),
        peer,
        closer: Some(Box::new(move || {
            let _ = closing.shutdown(Shutdown::Both);
        })),
    }
}

/// Connects to a WebSocket URL, `ws://host:port/path`.
#[derive(Debug, Clone)]
pub struct WsConnect {
    url: String,
    timeout: Duration,
}

impl WsConnect {
    /// To `url`, connecting and upgrading within [`CONNECT_TIMEOUT`].
    pub fn new(url: &str) -> WsConnect {
        WsConnect {
            url: url.to_string(),
            timeout: CONNECT_TIMEOUT,
        }
    }

    /// Within `timeout` per address tried, and again for the upgrade.
    pub fn timeout(mut self, timeout: Duration) -> WsConnect {
        self.timeout = timeout;
        self
    }

    fn address(&self) -> io::Result<String> {
        let invalid = |why: &str| io::Error::new(io::ErrorKind::InvalidInput, why.to_string());
        let uri: tungstenite::http::Uri = self
            .url
            .parse()
            .map_err(|_| invalid(&format!("`{}` is not a URL", self.url)))?;
        match uri.scheme_str() {
            Some("ws") => {}
            Some("wss") => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "wss:// is the secure level, which this build does not run",
                ));
            }
            _ => return Err(invalid(&format!("`{}` is not a ws:// URL", self.url))),
        }
        let host = uri
            .host()
            .ok_or_else(|| invalid(&format!("`{}` names no host", self.url)))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port = uri.port_u16().unwrap_or(80);
        Ok(if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        })
    }
}

impl Connect for WsConnect {
    fn connect(&self) -> io::Result<Connection> {
        let address = self.address()?;
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no address to connect to");
        let mut stream = None;
        for addr in address.to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, self.timeout) {
                Ok(s) => {
                    stream = Some((s, addr));
                    break;
                }
                Err(e) => last = e,
            }
        }
        let Some((stream, addr)) = stream else {
            return Err(last);
        };
        let socket = shared(stream)?;
        socket.stream.set_read_timeout(Some(self.timeout))?;
        socket.stream.set_write_timeout(Some(self.timeout))?;
        let (ws, _) = tungstenite::client::client_with_config(
            self.url.as_str(),
            socket.clone(),
            Some(config()),
        )
        .map_err(|e| match e {
            tungstenite::HandshakeError::Failure(e) => io_error(e),
            tungstenite::HandshakeError::Interrupted(_) => io::Error::new(
                io::ErrorKind::TimedOut,
                "the server's WebSocket upgrade did not arrive in time",
            ),
        })?;
        socket.stream.set_read_timeout(None)?;
        socket.stream.set_write_timeout(None)?;
        // The handshake's object reads on: it holds what arrived past the
        // upgrade.
        let mut ws = Box::new(ws);
        ws.get_mut().yields = true;
        Ok(connection(
            socket,
            format!("ws {addr}"),
            Reading::Open(ws),
            Role::Client,
        ))
    }

    fn describe(&self) -> String {
        format!("ws {}", self.url)
    }
}

/// A WebSocket listener that accepts only peers inside its ranges, as
/// [`TcpListen`] does. A connection upgrades on its first read, on the
/// thread serving it, so a client that connects and stalls holds only
/// that thread; a write before then is refused.
#[derive(Debug)]
pub struct WsListen {
    tcp: TcpListen,
}

impl WsListen {
    /// Binds `address` (a wildcard included), accepting peers inside
    /// `ranges`; an empty list is refused.
    pub fn bind(address: &str, ranges: &[Range]) -> io::Result<WsListen> {
        Ok(WsListen {
            tcp: TcpListen::bind(address, ranges)?,
        })
    }

    /// A listener the caller bound itself, under the same gate.
    pub fn from_listener(listener: TcpListener, ranges: &[Range]) -> io::Result<WsListen> {
        Ok(WsListen {
            tcp: TcpListen::from_listener(listener, ranges)?,
        })
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.tcp.local_addr()
    }

    /// The next connection, or a peer from outside the ranges, closed
    /// unread.
    pub fn accept(&self) -> io::Result<Accepted> {
        Ok(match self.tcp.accept_stream()? {
            Ok((stream, peer)) => Accepted::Connection(connection(
                shared(stream)?,
                format!("ws {peer}"),
                Reading::Upgrade,
                Role::Server,
            )),
            Err(refused) => Accepted::Refused(refused),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener() -> WsListen {
        WsListen::bind("127.0.0.1:0", &[Range::parse("127.0.0.1").unwrap()]).unwrap()
    }

    fn accepted(l: &WsListen) -> Connection {
        match l.accept().unwrap() {
            Accepted::Connection(c) => c,
            Accepted::Refused(peer) => panic!("{peer} refused"),
        }
    }

    /// A client's frame: `op`, FIN when `fin`, masked as RFC 6455 requires.
    fn masked(op: u8, fin: bool, payload: &[u8]) -> Vec<u8> {
        let key = [0x37, 0xfa, 0x21, 0x3d];
        let mut out = vec![if fin { 0x80 | op } else { op }];
        let n = payload.len();
        if n < 126 {
            out.push(0x80 | n as u8);
        } else {
            out.push(0x80 | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        out.extend_from_slice(&key);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        out
    }

    /// Sends RFC 6455's sample upgrade and answers the response's head.
    fn upgrade(raw: &mut TcpStream) -> String {
        raw.write_all(
            b"GET /chat HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\n\
              Connection: keep-alive, Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
              Origin: http://example.com\r\nSec-WebSocket-Version: 13\r\n\r\n",
        )
        .unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            raw.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        String::from_utf8(head).unwrap()
    }

    fn bytes(raw: &mut TcpStream, n: usize) -> Vec<u8> {
        let mut out = vec![0; n];
        raw.read_exact(&mut out).unwrap();
        out
    }

    #[test]
    fn a_browser_upgrade_is_answered_on_the_first_read_and_its_messages_arrive_whole() {
        let l = listener();
        let mut raw = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        raw.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        // Accepting reads nothing: a client that says nothing holds no one.
        let Connection {
            mut reader,
            mut writer,
            ..
        } = accepted(&l);
        let (tx, rx) = std::sync::mpsc::channel();
        let serving = std::thread::spawn(move || {
            let mut frame = Vec::new();
            for _ in 0..2 {
                assert!(reader.read_frame(&mut frame, 1 << 10).unwrap());
                tx.send(frame.clone()).unwrap();
            }
            reader.read_frame(&mut frame, 1 << 10)
        });
        let head = upgrade(&mut raw);
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        assert!(
            head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
            "RFC 6455's sample answer: {head}"
        );
        // A ping between the fragments of one message.
        raw.write_all(&masked(0x2, false, b"hel")).unwrap();
        raw.write_all(&masked(0x9, true, b"p")).unwrap();
        raw.write_all(&masked(0x0, true, b"lo")).unwrap();
        raw.write_all(&masked(0x2, true, b"x")).unwrap();
        let got = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(got, b"hello");
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), b"x");
        assert_eq!(bytes(&mut raw, 3), [0x8a, 1, b'p'], "the pong, unmasked");
        writer.write_frame(&[3, 0, 0, 0, 1, 2, 3]).unwrap();
        assert_eq!(
            bytes(&mut raw, 5),
            [0x82, 3, 1, 2, 3],
            "a frame is its body"
        );
        raw.write_all(&masked(0x1, true, b"text")).unwrap();
        let ended = serving.join().unwrap().unwrap_err();
        assert_eq!(ended.kind(), io::ErrorKind::InvalidData, "{ended}");
    }

    #[test]
    fn a_reader_answering_a_ping_never_waits_for_a_busy_writer() {
        let l = listener();
        let mut raw = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        raw.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let (stream, _) = l.tcp.accept_stream().unwrap().unwrap();
        let socket = shared(stream).unwrap();
        let writing = Arc::clone(&socket.writing);
        let mut conn = connection(socket, "test".into(), Reading::Upgrade, Role::Server);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut frame = Vec::new();
            while let Ok(true) = conn.reader.read_frame(&mut frame, 1 << 10) {
                if tx.send(frame.clone()).is_err() {
                    break;
                }
            }
        });
        upgrade(&mut raw);
        {
            // A writer inside a write the peer is not taking in.
            let _busy = writing.lock().unwrap();
            raw.write_all(&masked(0x9, true, b"p")).unwrap();
            raw.write_all(&masked(0x2, true, b"a")).unwrap();
            let got = rx.recv_timeout(Duration::from_secs(5));
            assert_eq!(got.as_deref(), Ok(&b"a"[..]), "the reader read on");
        }
        raw.write_all(&masked(0x2, true, b"b")).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), b"b");
        assert_eq!(bytes(&mut raw, 3), [0x8a, 1, b'p'], "the pong, once free");
    }

    #[test]
    fn a_close_ends_the_reads_and_is_answered() {
        let l = listener();
        let mut raw = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        raw.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut conn = accepted(&l);
        let serving = std::thread::spawn(move || {
            let mut frame = Vec::new();
            conn.reader.read_frame(&mut frame, 1 << 10)
        });
        upgrade(&mut raw);
        raw.write_all(&masked(0x8, true, &1000u16.to_be_bytes()))
            .unwrap();
        assert!(!serving.join().unwrap().unwrap(), "a close is a clean end");
        assert_eq!(bytes(&mut raw, 1), [0x88], "the close is answered");
    }

    #[test]
    fn an_unmasked_client_frame_is_refused() {
        let l = listener();
        let mut raw = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        raw.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut conn = accepted(&l);
        let serving = std::thread::spawn(move || {
            let mut frame = Vec::new();
            conn.reader.read_frame(&mut frame, 1 << 10)
        });
        upgrade(&mut raw);
        raw.write_all(&[0x82, 1, 7]).unwrap();
        assert!(serving.join().unwrap().is_err());
    }

    #[test]
    fn a_write_before_the_upgrade_is_refused() {
        let l = listener();
        let _raw = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let mut conn = accepted(&l);
        let refused = conn.writer.write_frame(&[1, 0, 0, 0, 9]).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::NotConnected);
    }

    #[test]
    fn a_client_and_a_server_carry_frames_both_ways_within_their_bound() {
        let l = listener();
        let addr = l.local_addr().unwrap();
        // A client that connects and says nothing, first.
        let _silent = TcpStream::connect(addr).unwrap();
        let _held = accepted(&l);
        let client = std::thread::spawn(move || {
            WsConnect::new(&format!("ws://{addr}/any/path"))
                .connect()
                .unwrap()
        });
        let mut server = accepted(&l);
        let (tx, rx) = std::sync::mpsc::channel();
        let reading = std::thread::spawn(move || {
            let mut frame = Vec::new();
            let mut got = Vec::new();
            for _ in 0..4 {
                assert!(server.reader.read_frame(&mut frame, 2 << 20).unwrap());
                got.push(frame.clone());
            }
            tx.send(got).unwrap();
            let over = server.reader.read_frame(&mut frame, 1 << 10);
            (server, over)
        });
        let mut client = client.join().unwrap();
        assert!(client.peer.starts_with("ws 127.0.0.1:"), "{}", client.peer);
        client
            .writer
            .write_frames(&[&[1, 0, 0, 0, 1], &[0, 0, 0, 0], &[2, 0, 0, 0, 2, 3]])
            .unwrap();
        let tail = vec![0xab; 1 << 20];
        let len = u32::try_from(1 + tail.len()).unwrap();
        let mut head = len.to_le_bytes().to_vec();
        head.push(7);
        client.writer.write_frame_parts(&head, &tail).unwrap();
        let got = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(got[0], [1]);
        assert!(got[1].is_empty());
        assert_eq!(got[2], [2, 3]);
        assert_eq!(got[3].len(), 1 + tail.len());
        assert_eq!(got[3][0], 7);
        client.writer.write_frame(&[0u8; 4 + 2048]).unwrap();
        let (mut server, over) = reading.join().unwrap();
        assert_eq!(over.unwrap_err().kind(), io::ErrorKind::InvalidData);
        server.writer.write_frame(&[2, 0, 0, 0, 4, 5]).unwrap();
        let mut back = Vec::new();
        assert!(client.reader.read_frame(&mut back, 1 << 10).unwrap());
        assert_eq!(back, [4, 5]);
    }

    #[test]
    fn a_url_that_is_not_ws_is_refused() {
        let kind = |url: &str| WsConnect::new(url).connect().unwrap_err().kind();
        assert_eq!(kind("wss://127.0.0.1:1/"), io::ErrorKind::Unsupported);
        assert_eq!(kind("tcp://127.0.0.1:1"), io::ErrorKind::InvalidInput);
        assert_eq!(kind("not a url"), io::ErrorKind::InvalidInput);
    }
}
