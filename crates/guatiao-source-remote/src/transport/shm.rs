// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The same machine, through shared memory: two single-producer byte
//! rings in a file both processes map, one per direction, with a local
//! connection beside them that carries the file's path once and then only
//! wake-ups and the peer's end.
//!
//! A reader polls its ring for a short while, then marks itself asleep,
//! looks once more, and sleeps on the connection until the writer rings
//! it with a byte. A busy link never sleeps; an idle one costs a wake-up
//! of the connection's kind. The ring's counters are written by the other
//! process, so every read of them is checked: a ring that claims more
//! than it holds ends the link.

use alloc::sync::Arc;
use core::cell::Cell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering, fence};
use core::time::Duration;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use super::deferred::deferred;
use super::local::{LocalConnect, LocalListen};
use super::stream::{StreamReader, StreamWriter};
use super::{Connect, Connection, FrameRead, FrameWrite};

/// Bytes each direction's ring holds.
pub const RING: u64 = 4 << 20;
/// A ring's counters, each on a cache line of its own, before its bytes.
const HEADER: u64 = 256;
const HEAD: u64 = 0;
const TAIL: u64 = 64;
const SLEEPING: u64 = 128;
/// The file: the client's ring to the server, then the server's back.
const FILE: u64 = 2 * (HEADER + RING);
/// How long a reader polls before it sleeps.
const SPIN: Duration = Duration::from_micros(50);

/// Memory both sides map, kept for as long as either ring is used.
struct Region {
    _map: memmap2::MmapMut,
    base: *mut u8,
}

// SAFETY: the region is plain memory both processes share; every access
// to it goes through atomics (the counters) or through byte copies of a
// span the counters hand to one side alone.
unsafe impl Send for Region {}
// SAFETY: as above.
unsafe impl Sync for Region {}

/// One direction's ring, over memory `base` points at: a header, then
/// `RING` bytes.
#[derive(Clone)]
struct Ring {
    /// Keeps the memory `base` points into alive.
    _region: Arc<dyn Holds>,
    base: *mut u8,
}

// SAFETY: as `Region`'s.
unsafe impl Send for Ring {}

/// Whatever keeps a ring's memory alive: a mapping, or a test's buffer.
trait Holds: Send + Sync {}
impl Holds for Region {}

impl Ring {
    fn counter(&self, at: u64) -> &AtomicU64 {
        // SAFETY: `at` is one of the header's counters, 8-aligned in a
        // page-aligned region that lives as long as `region`; the other
        // side touches it only atomically.
        unsafe { AtomicU64::from_ptr(self.base.add(at as usize).cast()) }
    }

    fn data(&self) -> *mut u8 {
        // SAFETY: the bytes follow the header in the same region.
        unsafe { self.base.add(HEADER as usize) }
    }

    /// What the reader may take: `head - tail`, refused when the other
    /// side's counters claim more than a ring holds.
    fn held(&self, head: u64, tail: u64) -> io::Result<u64> {
        let n = head.wrapping_sub(tail);
        if n > RING {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the shared ring's counters are not a ring's",
            ));
        }
        Ok(n)
    }

    /// Copies `n` bytes out from position `tail`.
    fn copy_out(&self, tail: u64, out: &mut [u8]) {
        let at = (tail % RING) as usize;
        let first = out.len().min(RING as usize - at);
        // SAFETY: both spans lie inside the ring's bytes, and the counters
        // give them to this side alone until `tail` moves past them.
        unsafe {
            core::ptr::copy_nonoverlapping(self.data().add(at), out.as_mut_ptr(), first);
            core::ptr::copy_nonoverlapping(
                self.data(),
                out.as_mut_ptr().add(first),
                out.len() - first,
            );
        }
    }

    /// Copies `bytes` in at position `head`.
    fn copy_in(&self, head: u64, bytes: &[u8]) {
        let at = (head % RING) as usize;
        let first = bytes.len().min(RING as usize - at);
        // SAFETY: as `copy_out`, for the span between head and the reader's
        // tail, which the reader does not read until `head` moves.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.data().add(at), first);
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr().add(first),
                self.data(),
                bytes.len() - first,
            );
        }
    }
}

/// The local connection beside the rings: a byte from the peer is a
/// wake-up, its end is the peer's.
struct Bell {
    read: BellRead,
    write: Arc<Mutex<Box<dyn Write + Send>>>,
}

/// Reads one direction's ring as a byte stream.
struct RingReader {
    ring: Ring,
    bell: BellRead,
    /// Set once the peer's end is seen: the writer stops waiting for room.
    gone: Arc<AtomicBool>,
    /// A read bound the link set, given to the bell on the next read: a
    /// ring the peer writes nothing into is waited on there.
    bound: Cell<Option<Option<Duration>>>,
}

/// Writes the other direction's ring as a byte stream.
struct RingWriter {
    ring: Ring,
    bell: Arc<Mutex<Box<dyn Write + Send>>>,
    gone: Arc<AtomicBool>,
    /// How long a write waits for room the reader frees none of.
    stall: Option<Duration>,
}

impl Read for RingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if let Some(bound) = self.bound.take() {
            self.bell.0.set_timeout(bound)?;
        }
        let tail = self.ring.counter(TAIL).load(Ordering::Relaxed);
        let started = Instant::now();
        let head = loop {
            let head = self.ring.counter(HEAD).load(Ordering::Acquire);
            if head != tail {
                break head;
            }
            if started.elapsed() < SPIN {
                core::hint::spin_loop();
                continue;
            }
            // Asleep, then a last look: a writer that published before it
            // saw the flag rings no bell, and this look finds its bytes.
            self.ring.counter(SLEEPING).store(1, Ordering::SeqCst);
            fence(Ordering::SeqCst);
            let head = self.ring.counter(HEAD).load(Ordering::SeqCst);
            let mut byte = [0u8; 1];
            if head != tail {
                if self.ring.counter(SLEEPING).swap(0, Ordering::SeqCst) == 0 {
                    // The writer cleared the flag: its byte is on the way.
                    self.wait_bell(&mut byte)?;
                }
                break head;
            }
            self.wait_bell(&mut byte)?;
        };
        let n = (self.ring.held(head, tail)? as usize).min(buf.len());
        self.ring.copy_out(tail, &mut buf[..n]);
        self.ring
            .counter(TAIL)
            .store(tail.wrapping_add(n as u64), Ordering::Release);
        Ok(n)
    }
}

impl RingReader {
    fn wait_bell(&mut self, byte: &mut [u8; 1]) -> io::Result<()> {
        match self.bell.read(byte) {
            Ok(1) => Ok(()),
            Ok(_) => {
                self.gone.store(true, Ordering::Release);
                Err(io::ErrorKind::UnexpectedEof.into())
            }
            Err(e) => {
                self.gone.store(true, Ordering::Release);
                Err(e)
            }
        }
    }
}

impl Write for RingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_vectored(&[io::IoSlice::new(bytes)])
    }

    /// Copies as much of `parts` as there is room for, in order, and
    /// publishes it once: one wake-up for all of it.
    fn write_vectored(&mut self, parts: &[io::IoSlice<'_>]) -> io::Result<usize> {
        let total: usize = parts.iter().map(|p| p.len()).sum();
        if total == 0 {
            return Ok(0);
        }
        let head = self.ring.counter(HEAD).load(Ordering::Relaxed);
        let mut waited = 0u32;
        let mut since = None;
        let room = loop {
            let tail = self.ring.counter(TAIL).load(Ordering::Acquire);
            let room = RING - self.ring.held(head, tail)?;
            if room > 0 {
                break room;
            }
            if self.gone.load(Ordering::Acquire) {
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
            stalled(&mut since, self.stall)?;
            if waited < 2000 {
                std::thread::yield_now();
            } else {
                std::thread::sleep(Duration::from_micros(200));
            }
        };
        let n = (room as usize).min(total);
        let mut done = 0usize;
        for part in parts {
            if done == n {
                break;
            }
            let take = part.len().min(n - done);
            self.ring
                .copy_in(head.wrapping_add(done as u64), &part[..take]);
            done += take;
        }
        self.ring
            .counter(HEAD)
            .store(head.wrapping_add(n as u64), Ordering::Release);
        fence(Ordering::SeqCst);
        if self.ring.counter(SLEEPING).load(Ordering::SeqCst) == 1
            && self.ring.counter(SLEEPING).swap(0, Ordering::SeqCst) == 1
        {
            let mut bell = self.bell.lock().unwrap_or_else(|e| e.into_inner());
            bell.write_all(&[1])?;
            bell.flush()?;
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Fails once `stall` has passed since the first call, which starts the
/// clock in `since`.
fn stalled(since: &mut Option<Instant>, stall: Option<Duration>) -> io::Result<()> {
    let Some(stall) = stall else {
        return Ok(());
    };
    let started = *since.get_or_insert_with(Instant::now);
    if started.elapsed() >= stall {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("the peer took nothing in for {stall:?}"),
        ));
    }
    Ok(())
}

fn map(path: &Path, create: bool) -> io::Result<Arc<Region>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true);
    if create {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Deleted once the last handle and mapping on it are gone; both
        // sides open it allowing that.
        const FILE_SHARE_ALL: u32 = 0x7;
        const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
        options
            .share_mode(FILE_SHARE_ALL)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
    }
    let file = options.open(path)?;
    if create {
        file.set_len(FILE)?;
    } else if file.metadata()?.len() < FILE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the shared file is shorter than two rings",
        ));
    }
    // SAFETY: a file this side created for the link, or the peer named
    // over the local connection; it is not truncated while mapped, and
    // every read of it is checked (`Ring::held`).
    let mut map = unsafe {
        memmap2::MmapOptions::new()
            .len(FILE as usize)
            .map_mut(&file)?
    };
    let base = map.as_mut_ptr();
    Ok(Arc::new(Region { _map: map, base }))
}

fn rings(region: &Arc<Region>, client: bool) -> (Ring, Ring) {
    let holder: Arc<dyn Holds> = region.clone();
    let to_server = Ring {
        _region: Arc::clone(&holder),
        base: region.base,
    };
    // SAFETY: the second ring starts inside the same mapping.
    let to_client_base = unsafe { region.base.add((HEADER + RING) as usize) };
    let to_client = Ring {
        _region: holder,
        base: to_client_base,
    };
    if client {
        (to_client, to_server)
    } else {
        (to_server, to_client)
    }
}

/// A connection over `region`'s rings, `bell` beside them.
fn connection(region: &Arc<Region>, client: bool, bell: Bell, peer: String) -> Connection {
    let (incoming, outgoing) = rings(region, client);
    let gone = Arc::new(AtomicBool::new(false));
    Connection {
        reader: Box::new(StreamReader::with_timeout(
            RingReader {
                ring: incoming,
                bell: bell.read,
                gone: Arc::clone(&gone),
                bound: Cell::new(None),
            },
            |r: &RingReader, bound| {
                r.bound.set(Some(bound));
                Ok(())
            },
        )),
        writer: Box::new(
            StreamWriter::new(RingWriter {
                ring: outgoing,
                bell: bell.write,
                gone: Arc::clone(&gone),
                stall: None,
            })
            .bounded_by(|w: &mut RingWriter, t| {
                w.stall = t;
                Ok(())
            }),
        ),
        peer,
        // A writer waiting for room stops waiting.
        closer: Some(Box::new(move || gone.store(true, Ordering::Release))),
    }
}

/// Splits a local connection into the bell's two ends.
fn bell_of(conn: Connection) -> (Box<dyn FrameRead>, Box<dyn FrameWrite>) {
    (conn.reader, conn.writer)
}

/// The bell over a local connection's frames: one frame of one byte per
/// ring, so its framing is the local transport's own.
struct BellRead(Box<dyn FrameRead>, Vec<u8>);

impl Read for BellRead {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.0.read_frame(&mut self.1, 16)? {
            out[0] = 1;
            Ok(1)
        } else {
            Ok(0)
        }
    }
}

struct BellWrite(Box<dyn FrameWrite>);

impl Write for BellWrite {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for _ in bytes {
            self.0.write_frame(&[1, 0, 0, 0, 1])?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bell(read: Box<dyn FrameRead>, write: Box<dyn FrameWrite>) -> Bell {
    Bell {
        read: BellRead(read, Vec::new()),
        write: Arc::new(Mutex::new(Box::new(BellWrite(write)))),
    }
}

/// Where a client puts its file: `/dev/shm` where there is one, else the
/// temporary directory.
fn shared_dir() -> PathBuf {
    let shm = Path::new("/dev/shm");
    if shm.is_dir() {
        shm.to_path_buf()
    } else {
        std::env::temp_dir()
    }
}

/// Connects to a local endpoint and moves the link onto shared memory.
#[derive(Debug, Clone)]
pub struct ShmConnect {
    local: LocalConnect,
    name: String,
}

impl ShmConnect {
    /// To the local endpoint `name` a [`ShmListen`] listens on.
    pub fn new(name: &str) -> ShmConnect {
        ShmConnect {
            local: LocalConnect::new(name),
            name: name.to_string(),
        }
    }
}

impl Connect for ShmConnect {
    fn connect(&self) -> io::Result<Connection> {
        let local = self.local.connect()?;
        let path = shared_dir().join(format!(
            "guatiao-remote-{}-{}.ring",
            std::process::id(),
            next_file()
        ));
        let region = map(&path, true)?;
        let (mut read, mut write) = bell_of(local);
        // The path once, as a frame; the server answers one byte when it
        // has mapped it.
        let path_text = path.display().to_string();
        let mut frame = (path_text.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(path_text.as_bytes());
        write.write_frame(&frame)?;
        read.set_timeout(Some(Duration::from_secs(10)))?;
        let mut ack = Vec::new();
        let acked = read.read_frame(&mut ack, 16)?;
        read.set_timeout(None)?;
        #[cfg(unix)]
        let _ = std::fs::remove_file(&path);
        if !acked || ack != [1] {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "the server did not map the shared memory",
            ));
        }
        Ok(connection(
            &region,
            true,
            bell(read, write),
            format!("shm {}", self.name),
        ))
    }

    fn describe(&self) -> String {
        format!("shm {}", self.name)
    }
}

/// Listens on a local endpoint for links that move onto shared memory.
#[derive(Debug)]
pub struct ShmListen {
    local: LocalListen,
}

impl ShmListen {
    /// Listens on the local endpoint `name`.
    pub fn bind(name: &str) -> io::Result<ShmListen> {
        Ok(ShmListen {
            local: LocalListen::bind(name)?,
        })
    }

    /// The next link. Its file is mapped on the link's first read, on the
    /// thread serving it, the client's path read within the bound the link
    /// set for its reads (10 s when none is set): accepting reads nothing.
    pub fn accept(&self) -> io::Result<Connection> {
        let mut local = self.local.accept()?;
        let peer = local.peer.clone();
        let abort = local.closer.take();
        let setup = {
            let peer = peer.clone();
            move |bound: Option<Duration>| {
                let (mut read, mut write) = bell_of(local);
                read.set_timeout(Some(bound.unwrap_or(Duration::from_secs(10))))?;
                let mut frame = Vec::new();
                if !read.read_frame(&mut frame, 4096)? {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                read.set_timeout(None)?;
                let path = core::str::from_utf8(&frame).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "a path that is not UTF-8")
                })?;
                let region = map(Path::new(path), false)?;
                write.write_frame(&[1, 0, 0, 0, 1])?;
                Ok(connection(&region, false, bell(read, write), peer))
            }
        };
        Ok(deferred(peer, Box::new(setup), abort))
    }
}

/// A process-wide counter naming each file this process creates: two
/// links of one process never share a file.
fn next_file() -> u64 {
    use core::sync::atomic::AtomicU64 as Counter;
    static NEXT: Counter = Counter::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ring over heap memory, as the mapping would hold it: owned by a
    /// raw pointer, so no move of an owner retags what the ring points at.
    struct Heap {
        words: *mut [u64],
    }

    // SAFETY: plain memory, freed once, by its one owner.
    unsafe impl Send for Heap {}
    // SAFETY: as above.
    unsafe impl Sync for Heap {}
    impl Holds for Heap {}

    impl Drop for Heap {
        fn drop(&mut self) {
            // SAFETY: the box `heap_ring` let go of, freed here alone.
            drop(unsafe { Box::from_raw(self.words) });
        }
    }

    fn heap_ring() -> Ring {
        let words = ((HEADER + RING) / 8) as usize;
        let raw = Box::into_raw(vec![0u64; words].into_boxed_slice());
        Ring {
            base: raw.cast::<u8>(),
            _region: Arc::new(Heap { words: raw }),
        }
    }

    #[test]
    fn bytes_cross_the_ring_in_order_across_its_end() {
        let ring = heap_ring();
        // Start near the end, so a copy wraps.
        let start = RING - 3;
        ring.counter(HEAD).store(start, Ordering::Relaxed);
        ring.counter(TAIL).store(start, Ordering::Relaxed);
        ring.copy_in(start, b"wrapped");
        ring.counter(HEAD).store(start + 7, Ordering::Release);
        let head = ring.counter(HEAD).load(Ordering::Acquire);
        let tail = ring.counter(TAIL).load(Ordering::Relaxed);
        let n = ring.held(head, tail).unwrap() as usize;
        let mut out = vec![0u8; n];
        ring.copy_out(tail, &mut out);
        assert_eq!(out, b"wrapped");
    }

    #[test]
    fn a_write_to_a_ring_nobody_drains_gives_up_at_its_bound() {
        let ring = heap_ring();
        // Full: the reader has taken nothing of a whole ring.
        ring.counter(HEAD).store(RING, Ordering::Relaxed);
        let mut writer = RingWriter {
            ring,
            bell: Arc::new(Mutex::new(Box::new(io::sink()))),
            gone: Arc::new(AtomicBool::new(false)),
            stall: Some(Duration::from_millis(100)),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(writer.write(b"more"));
        });
        let written = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the write gives up");
        assert_eq!(written.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    /// Counts the bytes rung.
    struct Rung(Arc<Mutex<usize>>);

    impl Write for Rung {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            *self.0.lock().unwrap() += buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn parts_are_copied_in_order_across_the_end_and_published_once() {
        let ring = heap_ring();
        // Five bytes from the end, with room for nine.
        let start = RING - 5;
        ring.counter(HEAD).store(start, Ordering::Relaxed);
        ring.counter(TAIL)
            .store(start - (RING - 9), Ordering::Relaxed);
        ring.counter(SLEEPING).store(1, Ordering::Relaxed);
        let rung = Arc::new(Mutex::new(0));
        let mut writer = RingWriter {
            ring,
            bell: Arc::new(Mutex::new(Box::new(Rung(Arc::clone(&rung))))),
            gone: Arc::new(AtomicBool::new(false)),
            stall: None,
        };
        let parts = [io::IoSlice::new(b"head"), io::IoSlice::new(b"tail-end")];
        assert_eq!(
            writer.write_vectored(&parts).unwrap(),
            9,
            "as much as there is room for"
        );
        assert_eq!(*rung.lock().unwrap(), 1, "one wake-up for both parts");
        let ring = &writer.ring;
        let head = ring.counter(HEAD).load(Ordering::Acquire);
        assert_eq!(head, start + 9);
        let mut out = vec![0u8; 9];
        ring.copy_out(start, &mut out);
        assert_eq!(out, b"headtail-");
    }

    #[test]
    fn counters_claiming_more_than_a_ring_are_refused() {
        let ring = heap_ring();
        assert!(ring.held(RING + 1, 0).is_err());
        assert_eq!(ring.held(RING, 0).unwrap(), RING);
        assert_eq!(ring.held(5, u64::MAX - 1).unwrap(), 7);
    }
}
