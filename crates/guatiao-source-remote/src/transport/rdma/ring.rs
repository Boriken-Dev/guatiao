// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Frames over RDMA: each side writes into the other's registered ring
//! with one-sided writes, and the TCP connection the link was set up over
//! stays open beside it as the peer's liveness.
//!
//! Each side registers an inbox (a [`RING`]-byte ring, then a counter)
//! and a staging area the same size. A writer copies a frame's bytes into
//! its staging area at the offset they will have in the peer's inbox and
//! posts an RDMA write with immediate data: the immediate is the byte
//! count, and it completes a receive on the peer, which is how the reader
//! learns that bytes arrived and sleeps on a completion channel while
//! none do. A reader tells the writer how far it has read by writing its
//! position into the counter after the writer's own inbox, every quarter
//! ring; a writer has room for what both that counter and its own
//! completions say is free. Positions are `u32` byte counts that wrap,
//! and the ring divides 2^32.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::fd::AsRawFd;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use super::verbs::{
    Completion, Device, IBV_WC_RECV_RDMA_WITH_IMM, IBV_WC_WR_FLUSH_ERR, Queue, Region, readable,
};
use super::{RING, RdmaPort};
use crate::transport::{Connection, FrameRead, FrameWrite};

/// The largest single write: a frame larger than this goes in parts the
/// reader starts on before the rest lands.
const CHUNK: usize = 256 << 10;
/// Requests the send queue holds: writes in flight and position updates.
const SENDS: u32 = 256;
/// Receives kept posted: writes that may land before the reader reposts.
const RECVS: u32 = 256;
/// How long a reader polls its completions before it sleeps.
const SPIN: Duration = Duration::from_micros(50);
/// How far a reader reads before it tells the writer.
const CREDIT_EVERY: u32 = (RING / 4) as u32;
/// Where the counter sits in an inbox, after the ring.
const COUNTER: usize = RING;
/// A request id marking a position update rather than a data write.
const CREDIT: u64 = 1 << 63;
/// What each side sends over TCP to set the link up.
const RECORD: usize = 64;
const MAGIC: &[u8; 4] = b"GTRD";
const VERSION: u8 = 1;
/// Both sides' queue pairs are ready to receive.
const READY: u8 = 0xA5;

/// What a side tells the other: where to write and how to reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Record {
    qpn: u32,
    psn: u32,
    gid: [u8; 16],
    mtu: u8,
    addr: u64,
    rkey: u32,
}

impl Record {
    fn encode(&self) -> [u8; RECORD] {
        let mut b = [0u8; RECORD];
        b[..4].copy_from_slice(MAGIC);
        b[4] = VERSION;
        b[5] = self.mtu;
        b[8..12].copy_from_slice(&self.qpn.to_le_bytes());
        b[12..16].copy_from_slice(&self.psn.to_le_bytes());
        b[16..32].copy_from_slice(&self.gid);
        b[32..40].copy_from_slice(&self.addr.to_le_bytes());
        b[40..44].copy_from_slice(&self.rkey.to_le_bytes());
        b[44..48].copy_from_slice(&(RING as u32).to_le_bytes());
        b
    }

    fn decode(b: &[u8; RECORD]) -> io::Result<Record> {
        let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_string());
        if &b[..4] != MAGIC {
            return Err(bad(
                "the peer is not an RDMA endpoint of guatiao's remote protocol",
            ));
        }
        if b[4] != VERSION {
            return Err(bad("the peer speaks another version of the RDMA set-up"));
        }
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        if u32_at(44) as usize != RING {
            return Err(bad("the peer's ring is not this side's size"));
        }
        let mtu = b[5];
        if !(1..=5).contains(&mtu) {
            return Err(bad("the peer names no MTU"));
        }
        let mut gid = [0u8; 16];
        gid.copy_from_slice(&b[16..32]);
        let mut addr = [0u8; 8];
        addr.copy_from_slice(&b[32..40]);
        Ok(Record {
            qpn: u32_at(8),
            psn: u32_at(12) & 0x00ff_ffff,
            gid,
            mtu,
            addr: u64::from_le_bytes(addr),
            rkey: u32_at(40),
        })
    }
}

/// The send queue's bookkeeping, shared by the writer (data) and the
/// reader (position updates).
struct Sends {
    outstanding: u32,
    /// The position up to which this side's data writes have completed.
    completed: u32,
    failed: Option<String>,
    done: Vec<Completion>,
}

/// What the reader, the writer and the closer share.
struct Shared {
    // Field order is drop order: the queue pair before the memory it
    // wrote from and to.
    queue: Queue,
    inbox: Region,
    staging: Region,
    _device: Arc<Device>,
    tcp: TcpStream,
    peer: Record,
    sends: Mutex<Sends>,
    gone: AtomicBool,
}

fn lock(m: &Mutex<Sends>) -> MutexGuard<'_, Sends> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Takes the send queue's completions; `s` is the lock held.
    fn reap(&self, s: &mut Sends) {
        let mut done = core::mem::take(&mut s.done);
        loop {
            done.clear();
            if let Err(e) = self.queue.poll_send(&mut done) {
                s.failed.get_or_insert_with(|| e.to_string());
                break;
            }
            if done.is_empty() {
                break;
            }
            for c in &done {
                s.outstanding = s.outstanding.saturating_sub(1);
                if c.status != 0 {
                    s.failed.get_or_insert_with(|| {
                        format!("an RDMA write failed (status {})", c.status)
                    });
                } else if c.wr_id & CREDIT == 0 {
                    s.completed = c.wr_id as u32;
                }
            }
        }
        s.done = done;
    }

    /// Posts through `post` once the send queue has room.
    fn post(&self, post: impl FnOnce(&Queue) -> io::Result<()>) -> io::Result<()> {
        let mut s = lock(&self.sends);
        loop {
            self.reap(&mut s);
            if let Some(why) = &s.failed {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, why.clone()));
            }
            if s.outstanding < SENDS {
                break;
            }
            if self.gone.load(Ordering::Acquire) {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            core::hint::spin_loop();
        }
        post(&self.queue)?;
        s.outstanding += 1;
        Ok(())
    }

    /// Ends the link both ways: the peer's side channel reads its end,
    /// and so does this side's reader.
    fn end(&self) {
        self.gone.store(true, Ordering::Release);
        let _ = self.tcp.shutdown(Shutdown::Both);
    }
}

struct Reader {
    shared: Arc<Shared>,
    /// Bytes that have landed in the inbox, and bytes read from it.
    head: u32,
    tail: u32,
    /// The position last written to the peer.
    told: u32,
    timeout: Option<Duration>,
    ended: bool,
    done: Vec<Completion>,
}

impl Reader {
    fn held(&self) -> io::Result<usize> {
        let n = self.head.wrapping_sub(self.tail) as usize;
        if n > RING {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the peer wrote more than the ring holds",
            ));
        }
        Ok(n)
    }

    /// Takes the receive queue's completions: whether any bytes landed.
    fn drain(&mut self) -> io::Result<bool> {
        let mut landed = false;
        loop {
            self.done.clear();
            self.shared.queue.poll_recv(&mut self.done)?;
            if self.done.is_empty() {
                return Ok(landed);
            }
            for i in 0..self.done.len() {
                let c = self.done[i];
                if c.status == IBV_WC_WR_FLUSH_ERR {
                    self.ended = true;
                    continue;
                }
                if c.status != 0 {
                    return Err(io::Error::other(format!(
                        "an RDMA receive failed (status {})",
                        c.status
                    )));
                }
                let (IBV_WC_RECV_RDMA_WITH_IMM, Some(n)) = (c.opcode, c.imm) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "the peer sent something other than a write with its length",
                    ));
                };
                self.head = self.head.wrapping_add(n);
                landed = true;
                self.shared.queue.post_recv(c.wr_id)?;
            }
            self.held()?;
        }
    }

    /// Waits until bytes are held: `false` at the link's end.
    fn fill(&mut self) -> io::Result<bool> {
        let started = Instant::now();
        loop {
            if self.held()? > 0 || self.drain()? {
                return Ok(true);
            }
            if self.ended {
                return Ok(false);
            }
            if started.elapsed() < SPIN {
                core::hint::spin_loop();
                continue;
            }
            // Armed, then a last look: a completion that came before the
            // arm raises no event, and this look finds it.
            self.shared.queue.arm_recv()?;
            if self.drain()? {
                return Ok(true);
            }
            let left = match self.timeout {
                None => None,
                Some(t) => match t.checked_sub(started.elapsed()) {
                    Some(left) => Some(left),
                    None => return Err(io::ErrorKind::TimedOut.into()),
                },
            };
            let fds = [self.shared.queue.channel_fd(), self.shared.tcp.as_raw_fd()];
            let Some([event, side]) = readable(fds, left)? else {
                return Err(io::ErrorKind::TimedOut.into());
            };
            if event {
                self.shared.queue.take_event()?;
            }
            if side {
                // The side channel carries nothing after set-up: a read is
                // its end, or this side's own shutdown. What the peer
                // wrote before it ended has completed here by now.
                let mut byte = [0u8; 1];
                let _ = (&self.shared.tcp).read(&mut byte);
                self.shared.gone.store(true, Ordering::Release);
                self.ended = true;
            }
        }
    }

    /// Copies held bytes into `out`: how many.
    fn take(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let at = self.tail as usize % RING;
        let n = self.held()?.min(out.len()).min(RING - at);
        self.shared.inbox.copy_out(at, &mut out[..n]);
        self.tail = self.tail.wrapping_add(n as u32);
        if self.tail.wrapping_sub(self.told) >= CREDIT_EVERY {
            self.told = self.tail;
            let bytes = self.tail.to_le_bytes();
            let remote = (
                self.shared.peer.addr + COUNTER as u64,
                self.shared.peer.rkey,
            );
            self.shared
                .post(|q| q.post_write_inline(CREDIT, &bytes, remote))?;
        }
        Ok(n)
    }

    fn read_exact(&mut self, out: &mut [u8]) -> io::Result<()> {
        let mut got = 0;
        while got < out.len() {
            if self.held()? == 0 && !self.fill()? {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            got += self.take(&mut out[got..])?;
        }
        Ok(())
    }
}

impl FrameRead for Reader {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        // A clean end is one between frames.
        if self.held()? == 0 && !self.fill()? {
            return Ok(false);
        }
        let mut len = [0u8; 4];
        self.read_exact(&mut len)?;
        let n = u32::from_le_bytes(len) as usize;
        if n > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("a frame of {n} bytes is over the {max} allowed"),
            ));
        }
        buf.clear();
        buf.resize(n, 0);
        self.read_exact(buf)?;
        Ok(true)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout;
        Ok(())
    }
}

struct Writer {
    shared: Arc<Shared>,
    /// Bytes written to the peer's inbox.
    head: u32,
    /// How long a write waits for room the reader frees none of.
    stall: Option<Duration>,
}

impl Writer {
    /// Room in the peer's inbox and in staging, waiting while there is
    /// none.
    fn room(&self) -> io::Result<usize> {
        let counter = self.shared.inbox.word(COUNTER);
        let mut waited = 0u32;
        let mut since: Option<Instant> = None;
        loop {
            let completed = {
                let mut s = lock(&self.shared.sends);
                self.shared.reap(&mut s);
                if let Some(why) = &s.failed {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, why.clone()));
                }
                s.completed
            };
            let read = self.head.wrapping_sub(counter.load(Ordering::Acquire)) as usize;
            if read > RING {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the peer's position is not one this side wrote",
                ));
            }
            let used = read.max(self.head.wrapping_sub(completed) as usize);
            if used < RING {
                return Ok(RING - used);
            }
            if self.shared.gone.load(Ordering::Acquire) {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            // A full ring drains as fast as the reader runs: poll, then
            // yield, then sleep in short steps.
            waited += 1;
            if waited < 1000 {
                core::hint::spin_loop();
                continue;
            }
            // A yield on a loaded machine can last a whole time slice.
            if let Some(stall) = self.stall
                && since.get_or_insert_with(Instant::now).elapsed() >= stall
            {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("the peer took nothing in for {stall:?}"),
                ));
            }
            if waited < 2000 {
                std::thread::yield_now();
            } else {
                std::thread::sleep(Duration::from_micros(200));
            }
        }
    }

    /// Writes part of `bytes`: how much.
    fn write_some(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.shared.gone.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let room = self.room()?;
        let at = self.head as usize % RING;
        let n = room.min(bytes.len()).min(RING - at).min(CHUNK);
        self.shared.staging.copy_in(at, &bytes[..n]);
        let end = self.head.wrapping_add(n as u32);
        let remote = (self.shared.peer.addr + at as u64, self.shared.peer.rkey);
        let staging = &self.shared.staging;
        self.shared
            .post(|q| q.post_write(u64::from(end), staging, at, n, remote, Some(n as u32)))?;
        self.head = end;
        Ok(n)
    }

    fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let n = self.write_some(bytes)?;
            bytes = &bytes[n..];
        }
        Ok(())
    }
}

impl FrameWrite for Writer {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.write_all(frame)
    }

    fn write_frame_parts(&mut self, head: &[u8], tail: &[u8]) -> io::Result<()> {
        self.write_all(head)?;
        self.write_all(tail)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.stall = timeout;
        Ok(())
    }

    fn shutdown(&mut self) {
        // What was posted lands before the side channel says the link
        // ended, within a bound.
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            {
                let mut s = lock(&self.shared.sends);
                self.shared.reap(&mut s);
                if s.outstanding == 0 || s.failed.is_some() {
                    break;
                }
            }
            if Instant::now() > deadline {
                break;
            }
            std::thread::yield_now();
        }
        self.shared.end();
    }
}

fn random_psn() -> io::Result<u32> {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(u32::from_le_bytes(b) & 0x00ff_ffff)
}

/// Sets a link up over `tcp`, connected to the peer, and keeps `tcp`
/// beside it.
pub(super) fn establish(
    tcp: TcpStream,
    port: &RdmaPort,
    timeout: Duration,
    peer_name: String,
) -> io::Result<Connection> {
    tcp.set_nodelay(true)?;
    let device = Device::open(port.device.as_deref(), port.port, port.gid_index)?;
    let queue = Queue::new(&device, SENDS, RECVS)?;
    let inbox = Region::new(&device, COUNTER + 64, true)?;
    let staging = Region::new(&device, RING, false)?;
    queue.to_init()?;
    for i in 0..u64::from(RECVS) {
        queue.post_recv(i)?;
    }
    let psn = random_psn()?;
    let mine = Record {
        qpn: queue.qpn,
        psn,
        gid: device.gid,
        mtu: device.mtu,
        addr: inbox.addr(),
        rkey: inbox.rkey,
    };
    tcp.set_read_timeout(Some(timeout))?;
    tcp.set_write_timeout(Some(timeout))?;
    (&tcp).write_all(&mine.encode())?;
    let mut theirs = [0u8; RECORD];
    (&tcp).read_exact(&mut theirs)?;
    let peer = Record::decode(&theirs)?;
    queue.to_rtr(peer.qpn, peer.psn, &peer.gid, mine.mtu.min(peer.mtu))?;
    queue.to_rts(psn)?;
    // Neither side writes before the other can receive.
    (&tcp).write_all(&[READY])?;
    let mut ready = [0u8; 1];
    (&tcp).read_exact(&mut ready)?;
    if ready[0] != READY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the peer did not finish setting the link up",
        ));
    }
    tcp.set_read_timeout(None)?;
    tcp.set_write_timeout(None)?;
    let peer_name = format!("{peer_name} through {}", device.name);
    let shared = Arc::new(Shared {
        queue,
        inbox,
        staging,
        _device: device,
        tcp,
        peer,
        sends: Mutex::new(Sends {
            outstanding: 0,
            completed: 0,
            failed: None,
            done: Vec::new(),
        }),
        gone: AtomicBool::new(false),
    });
    let closing = Arc::clone(&shared);
    Ok(Connection {
        reader: Box::new(Reader {
            shared: Arc::clone(&shared),
            head: 0,
            tail: 0,
            told: 0,
            timeout: None,
            ended: false,
            done: Vec::new(),
        }),
        writer: Box::new(Writer {
            shared,
            head: 0,
            stall: None,
        }),
        peer: peer_name,
        closer: Some(Box::new(move || closing.end())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_reads_back_as_written_and_a_stranger_is_refused() {
        let r = Record {
            qpn: 0x0001_2345,
            psn: 0x00ab_cdef,
            gid: [7; 16],
            mtu: 5,
            addr: 0x7f00_1234_5000,
            rkey: 0xdead_beef,
        };
        assert_eq!(Record::decode(&r.encode()).unwrap(), r);
        let mut other = r.encode();
        other[0] = b'X';
        assert!(Record::decode(&other).is_err());
        let mut later = r.encode();
        later[4] = 2;
        assert!(Record::decode(&later).is_err());
        let mut smaller = r.encode();
        smaller[44..48].copy_from_slice(&(1u32 << 16).to_le_bytes());
        assert!(Record::decode(&smaller).is_err());
    }

    #[test]
    fn positions_wrap_where_the_ring_does() {
        // 2^32 is a whole number of rings, so an offset taken from a
        // wrapped position is the one it had before wrapping.
        assert_eq!((1u64 << 32) % RING as u64, 0);
        let near = u32::MAX - 9;
        assert_eq!(near.wrapping_add(20) as usize % RING, 10);
        assert_eq!(near.wrapping_add(20).wrapping_sub(near), 20);
    }
}
