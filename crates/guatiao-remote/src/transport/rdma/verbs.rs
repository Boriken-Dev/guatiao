// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The verbs this transport calls, from `libibverbs.so.1` opened at run
//! time, so building needs neither rdma-core's headers nor its library.
//!
//! The structs read and filled here mirror `infiniband/verbs.h` (rdma-core
//! 61; the layout has been stable since the library's 1.1 symbol versions),
//! and the sizes and offsets below are the header's own. Every pointer and
//! status the library hands back is checked here; the rest of the
//! transport sees safe types.

use alloc::alloc::{alloc_zeroed, dealloc};
use alloc::sync::Arc;
use core::alloc::Layout;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::mem::{offset_of, size_of};
use core::ptr::{NonNull, null_mut};
use core::sync::atomic::AtomicU32;
use core::time::Duration;
use std::io;
use std::os::fd::RawFd;

const IBV_QPS_INIT: u32 = 1;
const IBV_QPS_RTR: u32 = 2;
const IBV_QPS_RTS: u32 = 3;
const IBV_QPT_RC: c_int = 2;
const IBV_PORT_ACTIVE: u32 = 4;
const IBV_ACCESS_LOCAL_WRITE: c_int = 1;
const IBV_ACCESS_REMOTE_WRITE: c_int = 2;
const IBV_QP_STATE: c_int = 1;
const IBV_QP_ACCESS_FLAGS: c_int = 8;
const IBV_QP_PKEY_INDEX: c_int = 16;
const IBV_QP_PORT: c_int = 32;
const IBV_QP_AV: c_int = 128;
const IBV_QP_PATH_MTU: c_int = 256;
const IBV_QP_TIMEOUT: c_int = 512;
const IBV_QP_RETRY_CNT: c_int = 1024;
const IBV_QP_RNR_RETRY: c_int = 2048;
const IBV_QP_RQ_PSN: c_int = 4096;
const IBV_QP_MAX_QP_RD_ATOMIC: c_int = 8192;
const IBV_QP_MIN_RNR_TIMER: c_int = 32768;
const IBV_QP_SQ_PSN: c_int = 65536;
const IBV_QP_MAX_DEST_RD_ATOMIC: c_int = 131_072;
const IBV_QP_DEST_QPN: c_int = 1_048_576;
const IBV_WR_RDMA_WRITE: u32 = 0;
const IBV_WR_RDMA_WRITE_WITH_IMM: u32 = 1;
const IBV_SEND_SIGNALED: c_uint = 2;
const IBV_SEND_INLINE: c_uint = 8;
const IBV_WC_WITH_IMM: u32 = 2;
/// The completion status of work flushed from a queue pair in error (as
/// after a write to a peer that is gone runs out of retries).
pub(super) const IBV_WC_WR_FLUSH_ERR: u32 = 5;
/// The completion opcode of a write with immediate data, received.
pub(super) const IBV_WC_RECV_RDMA_WITH_IMM: u32 = 129;
/// The most inline data a send carries.
const INLINE: u32 = 16;

/// `struct ibv_context` up to its operations table, which holds the calls
/// `verbs.h` makes from inline functions.
#[repr(C)]
struct Context {
    device: *mut c_void,
    ops: [*const c_void; 32],
}

/// Indexes into `ops`: `poll_cq`, `req_notify_cq`, `post_send`, `post_recv`.
const OP_POLL_CQ: usize = 11;
const OP_REQ_NOTIFY_CQ: usize = 12;
const OP_POST_SEND: usize = 25;
const OP_POST_RECV: usize = 26;

#[repr(C)]
struct CompChannel {
    context: *mut c_void,
    fd: c_int,
    refcnt: c_int,
}

#[repr(C)]
struct Mr {
    context: *mut c_void,
    pd: *mut c_void,
    addr: *mut c_void,
    length: usize,
    handle: u32,
    lkey: u32,
    rkey: u32,
}

/// `struct ibv_qp` up to its number.
#[repr(C)]
struct Qp {
    context: *mut c_void,
    qp_context: *mut c_void,
    pd: *mut c_void,
    send_cq: *mut c_void,
    recv_cq: *mut c_void,
    srq: *mut c_void,
    handle: u32,
    qp_num: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct QpCap {
    max_send_wr: u32,
    max_recv_wr: u32,
    max_send_sge: u32,
    max_recv_sge: u32,
    max_inline_data: u32,
}

#[repr(C)]
struct QpInitAttr {
    qp_context: *mut c_void,
    send_cq: *mut c_void,
    recv_cq: *mut c_void,
    srq: *mut c_void,
    cap: QpCap,
    qp_type: c_int,
    sq_sig_all: c_int,
}

/// `union ibv_gid` is 8-aligned (it overlays two `__be64`).
#[repr(C, align(8))]
#[derive(Default, Clone, Copy)]
struct GlobalRoute {
    dgid: [u8; 16],
    flow_label: u32,
    sgid_index: u8,
    hop_limit: u8,
    traffic_class: u8,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct AhAttr {
    grh: GlobalRoute,
    dlid: u16,
    sl: u8,
    src_path_bits: u8,
    static_rate: u8,
    is_global: u8,
    port_num: u8,
}

#[repr(C)]
#[derive(Default)]
struct QpAttr {
    qp_state: u32,
    cur_qp_state: u32,
    path_mtu: u32,
    path_mig_state: u32,
    qkey: u32,
    rq_psn: u32,
    sq_psn: u32,
    dest_qp_num: u32,
    qp_access_flags: c_uint,
    cap: QpCap,
    ah_attr: AhAttr,
    alt_ah_attr: AhAttr,
    pkey_index: u16,
    alt_pkey_index: u16,
    en_sqd_async_notify: u8,
    sq_draining: u8,
    max_rd_atomic: u8,
    max_dest_rd_atomic: u8,
    min_rnr_timer: u8,
    port_num: u8,
    timeout: u8,
    retry_cnt: u8,
    rnr_retry: u8,
    alt_port_num: u8,
    alt_timeout: u8,
    rate_limit: u32,
}

#[repr(C)]
struct Sge {
    addr: u64,
    length: u32,
    lkey: u32,
}

/// `struct ibv_send_wr`, its `wr` union read as `rdma`; the unions after
/// it are left zero.
#[repr(C)]
struct SendWr {
    wr_id: u64,
    next: *mut SendWr,
    sg_list: *mut Sge,
    num_sge: c_int,
    opcode: u32,
    send_flags: c_uint,
    imm_data: u32,
    remote_addr: u64,
    rkey: u32,
    wr_rest: [u32; 5],
    tail: [u64; 7],
}

#[repr(C)]
struct RecvWr {
    wr_id: u64,
    next: *mut RecvWr,
    sg_list: *mut Sge,
    num_sge: c_int,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Wc {
    wr_id: u64,
    status: u32,
    opcode: u32,
    vendor_err: u32,
    byte_len: u32,
    imm_data: u32,
    qp_num: u32,
    src_qp: u32,
    wc_flags: u32,
    pkey_index: u16,
    slid: u16,
    sl: u8,
    dlid_path_bits: u8,
}

// The header's sizes and offsets on x86-64 and aarch64 Linux.
const _: () = {
    assert!(offset_of!(Context, ops) == 8);
    assert!(offset_of!(Context, ops) + OP_POLL_CQ * 8 == 96);
    assert!(offset_of!(Context, ops) + OP_REQ_NOTIFY_CQ * 8 == 104);
    assert!(offset_of!(Context, ops) + OP_POST_SEND * 8 == 208);
    assert!(offset_of!(Context, ops) + OP_POST_RECV * 8 == 216);
    assert!(size_of::<CompChannel>() == 16 && offset_of!(CompChannel, fd) == 8);
    assert!(size_of::<Mr>() == 48 && offset_of!(Mr, lkey) == 36 && offset_of!(Mr, rkey) == 40);
    assert!(offset_of!(Qp, qp_num) == 52);
    assert!(size_of::<QpCap>() == 20);
    assert!(size_of::<QpInitAttr>() == 64 && offset_of!(QpInitAttr, qp_type) == 52);
    assert!(size_of::<GlobalRoute>() == 24 && offset_of!(GlobalRoute, sgid_index) == 20);
    assert!(size_of::<AhAttr>() == 32 && offset_of!(AhAttr, is_global) == 29);
    assert!(offset_of!(AhAttr, port_num) == 30);
    assert!(size_of::<QpAttr>() == 144);
    assert!(offset_of!(QpAttr, cap) == 36 && offset_of!(QpAttr, ah_attr) == 56);
    assert!(offset_of!(QpAttr, pkey_index) == 120 && offset_of!(QpAttr, max_rd_atomic) == 126);
    assert!(offset_of!(QpAttr, min_rnr_timer) == 128 && offset_of!(QpAttr, rnr_retry) == 132);
    assert!(offset_of!(QpAttr, rate_limit) == 136);
    assert!(size_of::<Sge>() == 16);
    assert!(size_of::<SendWr>() == 128 && offset_of!(SendWr, imm_data) == 36);
    assert!(offset_of!(SendWr, remote_addr) == 40 && offset_of!(SendWr, rkey) == 48);
    assert!(size_of::<RecvWr>() == 32);
    assert!(size_of::<Wc>() == 48 && offset_of!(Wc, imm_data) == 24);
    assert!(offset_of!(Wc, wc_flags) == 36);
};

type PollCq = unsafe extern "C" fn(*mut c_void, c_int, *mut Wc) -> c_int;
type ReqNotifyCq = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type PostSend = unsafe extern "C" fn(*mut c_void, *mut SendWr, *mut *mut SendWr) -> c_int;
type PostRecv = unsafe extern "C" fn(*mut c_void, *mut RecvWr, *mut *mut RecvWr) -> c_int;

/// The library and the exported calls.
struct Verbs {
    get_device_list: unsafe extern "C" fn(*mut c_int) -> *mut *mut c_void,
    free_device_list: unsafe extern "C" fn(*mut *mut c_void),
    get_device_name: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    open_device: unsafe extern "C" fn(*mut c_void) -> *mut Context,
    close_device: unsafe extern "C" fn(*mut Context) -> c_int,
    query_port: unsafe extern "C" fn(*mut Context, u8, *mut c_void) -> c_int,
    query_gid: unsafe extern "C" fn(*mut Context, u8, c_int, *mut [u8; 16]) -> c_int,
    alloc_pd: unsafe extern "C" fn(*mut Context) -> *mut c_void,
    dealloc_pd: unsafe extern "C" fn(*mut c_void) -> c_int,
    reg_mr: unsafe extern "C" fn(*mut c_void, *mut c_void, usize, c_int) -> *mut Mr,
    dereg_mr: unsafe extern "C" fn(*mut Mr) -> c_int,
    create_comp_channel: unsafe extern "C" fn(*mut Context) -> *mut CompChannel,
    destroy_comp_channel: unsafe extern "C" fn(*mut CompChannel) -> c_int,
    create_cq: unsafe extern "C" fn(
        *mut Context,
        c_int,
        *mut c_void,
        *mut CompChannel,
        c_int,
    ) -> *mut c_void,
    destroy_cq: unsafe extern "C" fn(*mut c_void) -> c_int,
    get_cq_event:
        unsafe extern "C" fn(*mut CompChannel, *mut *mut c_void, *mut *mut c_void) -> c_int,
    ack_cq_events: unsafe extern "C" fn(*mut c_void, c_uint),
    create_qp: unsafe extern "C" fn(*mut c_void, *mut QpInitAttr) -> *mut Qp,
    modify_qp: unsafe extern "C" fn(*mut Qp, *mut QpAttr, c_int) -> c_int,
    destroy_qp: unsafe extern "C" fn(*mut Qp) -> c_int,
    /// Keeps the calls above valid.
    _lib: libloading::Library,
}

/// What a verb's status says: a positive errno, or -1 with `errno` set.
fn status(rc: c_int, what: &str) -> io::Result<()> {
    if rc == 0 {
        return Ok(());
    }
    let e = if rc > 0 {
        io::Error::from_raw_os_error(rc)
    } else {
        io::Error::last_os_error()
    };
    Err(io::Error::new(e.kind(), format!("{what}: {e}")))
}

/// A pointer a verb returned, or the `errno` it set.
fn made<T>(p: *mut T, what: &str) -> io::Result<NonNull<T>> {
    NonNull::new(p).ok_or_else(|| {
        let e = io::Error::last_os_error();
        io::Error::new(e.kind(), format!("{what}: {e}"))
    })
}

impl Verbs {
    fn load() -> io::Result<Arc<Verbs>> {
        // SAFETY: rdma-core's libibverbs, whose initialisers only read its
        // configuration and providers' directory.
        let lib = unsafe { libloading::Library::new("libibverbs.so.1") }.map_err(|e| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("libibverbs.so.1 (rdma-core) could not be loaded: {e}"),
            )
        })?;
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: each call is declared with the signature verbs.h
                // gives it, and kept no longer than `lib`.
                *unsafe { lib.get(concat!($name, "\0").as_bytes()) }.map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("libibverbs.so.1 has no `{}`: {e}", $name),
                    )
                })?
            };
        }
        Ok(Arc::new(Verbs {
            get_device_list: sym!("ibv_get_device_list"),
            free_device_list: sym!("ibv_free_device_list"),
            get_device_name: sym!("ibv_get_device_name"),
            open_device: sym!("ibv_open_device"),
            close_device: sym!("ibv_close_device"),
            query_port: sym!("ibv_query_port"),
            query_gid: sym!("ibv_query_gid"),
            alloc_pd: sym!("ibv_alloc_pd"),
            dealloc_pd: sym!("ibv_dealloc_pd"),
            reg_mr: sym!("ibv_reg_mr"),
            dereg_mr: sym!("ibv_dereg_mr"),
            create_comp_channel: sym!("ibv_create_comp_channel"),
            destroy_comp_channel: sym!("ibv_destroy_comp_channel"),
            create_cq: sym!("ibv_create_cq"),
            destroy_cq: sym!("ibv_destroy_cq"),
            get_cq_event: sym!("ibv_get_cq_event"),
            ack_cq_events: sym!("ibv_ack_cq_events"),
            create_qp: sym!("ibv_create_qp"),
            modify_qp: sym!("ibv_modify_qp"),
            destroy_qp: sym!("ibv_destroy_qp"),
            _lib: lib,
        }))
    }

    /// Every device's name, and the device the caller picks, or none.
    fn devices<T>(
        &self,
        pick: impl FnMut(&str, *mut c_void) -> Option<T>,
    ) -> io::Result<(Vec<String>, Option<T>)> {
        let mut pick = pick;
        let mut n: c_int = 0;
        // SAFETY: writes the count it is given.
        let list = made(
            unsafe { (self.get_device_list)(&mut n) },
            "listing RDMA devices",
        )?;
        let mut names = Vec::new();
        let mut picked = None;
        for i in 0..usize::try_from(n).unwrap_or(0) {
            // SAFETY: the list holds `n` devices, then a null.
            let dev = unsafe { *list.as_ptr().add(i) };
            if dev.is_null() {
                break;
            }
            // SAFETY: a device of the list; its name lives as long as it.
            let name = unsafe { (self.get_device_name)(dev) };
            if name.is_null() {
                continue;
            }
            // SAFETY: a NUL-terminated name, read before the list is freed.
            let name = unsafe { CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned();
            if picked.is_none() {
                picked = pick(&name, dev);
            }
            names.push(name);
        }
        // SAFETY: the list this call made; an opened device outlives it.
        unsafe { (self.free_device_list)(list.as_ptr()) };
        Ok((names, picked))
    }
}

/// The names of the RDMA devices libibverbs lists.
pub(super) fn device_names() -> io::Result<Vec<String>> {
    Ok(Verbs::load()?.devices(|_, _| None::<()>)?.0)
}

/// An opened device, its protection domain, and the port and address
/// links go through.
pub(super) struct Device {
    verbs: Arc<Verbs>,
    ctx: NonNull<Context>,
    pd: NonNull<c_void>,
    poll_cq: PollCq,
    req_notify_cq: ReqNotifyCq,
    post_send: PostSend,
    post_recv: PostRecv,
    pub(super) name: String,
    pub(super) port: u8,
    pub(super) gid_index: u8,
    pub(super) gid: [u8; 16],
    /// The port's active MTU, as `enum ibv_mtu` numbers it (1 is 256
    /// bytes, 5 is 4096).
    pub(super) mtu: u8,
}

// SAFETY: libibverbs' objects may be used from any thread; a completion
// queue is polled by one thread at a time (the callers' locks).
unsafe impl Send for Device {}
// SAFETY: as above.
unsafe impl Sync for Device {}

impl Device {
    /// Opens `name`, or the first device, at `port`; the address is
    /// `gid_index`, or the first IPv4 one (RoCE v2 over IPv4), else 0.
    pub(super) fn open(
        name: Option<&str>,
        port: u8,
        gid_index: Option<u8>,
    ) -> io::Result<Arc<Device>> {
        let verbs = Verbs::load()?;
        let (names, opened) = verbs.devices(|n, dev| {
            if name.is_some_and(|want| want != n) {
                return None;
            }
            // SAFETY: a device of the live list.
            Some((n.to_string(), unsafe { (verbs.open_device)(dev) }))
        })?;
        let Some((dev_name, ctx)) = opened else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                match name {
                    Some(want) => format!("no RDMA device `{want}` (libibverbs lists {names:?})"),
                    None => "no RDMA device: libibverbs lists none (for Soft-RoCE, load \
                             rdma_rxe and add one with `rdma link add`)"
                        .to_string(),
                },
            ));
        };
        let ctx = made(ctx, "opening the RDMA device")?;
        // SAFETY: the context is open; its operations table is filled by
        // the provider when it opens and never changes after.
        let ops = unsafe { (*ctx.as_ptr()).ops };
        let close = || {
            // SAFETY: closed once, on this failure path alone.
            unsafe { (verbs.close_device)(ctx.as_ptr()) };
        };
        let (Some(poll_cq), Some(req_notify_cq), Some(post_send), Some(post_recv)) = (
            NonNull::new(ops[OP_POLL_CQ].cast_mut()),
            NonNull::new(ops[OP_REQ_NOTIFY_CQ].cast_mut()),
            NonNull::new(ops[OP_POST_SEND].cast_mut()),
            NonNull::new(ops[OP_POST_RECV].cast_mut()),
        ) else {
            close();
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("the provider of `{dev_name}` fills no data-path calls"),
            ));
        };
        // SAFETY: the provider's entries of `struct ibv_context_ops`, of
        // the types verbs.h declares for them.
        let (poll_cq, req_notify_cq, post_send, post_recv) = unsafe {
            (
                core::mem::transmute::<*mut c_void, PollCq>(poll_cq.as_ptr()),
                core::mem::transmute::<*mut c_void, ReqNotifyCq>(req_notify_cq.as_ptr()),
                core::mem::transmute::<*mut c_void, PostSend>(post_send.as_ptr()),
                core::mem::transmute::<*mut c_void, PostRecv>(post_recv.as_ptr()),
            )
        };
        // `struct ibv_port_attr` is 56 bytes; the exported call fills its
        // older, shorter form. Read: `state` at 0, `active_mtu` at 8.
        let mut attr = [0u32; 64];
        // SAFETY: a buffer larger than either form of the struct.
        if let Err(e) = status(
            unsafe { (verbs.query_port)(ctx.as_ptr(), port, attr.as_mut_ptr().cast()) },
            "querying the RDMA port",
        ) {
            close();
            return Err(e);
        }
        if attr[0] != IBV_PORT_ACTIVE {
            close();
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                format!("port {port} of `{dev_name}` is not active"),
            ));
        }
        let mtu = u8::try_from(attr[2].clamp(1, 5)).unwrap_or(1);
        let gid_at = |i: u8| {
            let mut gid = [0u8; 16];
            // SAFETY: writes one GID into the 16 bytes it is given.
            let rc = unsafe { (verbs.query_gid)(ctx.as_ptr(), port, c_int::from(i), &mut gid) };
            (rc == 0).then_some(gid)
        };
        let ipv4 = |g: &[u8; 16]| g[..10].iter().all(|&b| b == 0) && g[10..12] == [0xff, 0xff];
        let found = match gid_index {
            Some(i) => gid_at(i).map(|g| (i, g)),
            None => (0..16u8)
                .map_while(|i| gid_at(i).map(|g| (i, g)))
                .find(|(_, g)| ipv4(g))
                .or_else(|| gid_at(0).map(|g| (0, g))),
        };
        let Some((gid_index, gid)) = found else {
            close();
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("port {port} of `{dev_name}` has no such address (GID)"),
            ));
        };
        // SAFETY: the open context.
        let pd = match made(
            unsafe { (verbs.alloc_pd)(ctx.as_ptr()) },
            "allocating a protection domain",
        ) {
            Ok(pd) => pd,
            Err(e) => {
                close();
                return Err(e);
            }
        };
        Ok(Arc::new(Device {
            verbs,
            ctx,
            pd,
            poll_cq,
            req_notify_cq,
            post_send,
            post_recv,
            name: dev_name,
            port,
            gid_index,
            gid,
            mtu,
        }))
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: every region and queue holds the device, so none is
        // left; each is released once.
        unsafe {
            (self.verbs.dealloc_pd)(self.pd.as_ptr());
            (self.verbs.close_device)(self.ctx.as_ptr());
        }
    }
}

/// Page-aligned memory registered with the device: a remote peer may
/// write it (`remote_write`), so it is read and written only through raw
/// copies and atomics, never through a reference.
pub(super) struct Region {
    dev: Arc<Device>,
    ptr: NonNull<u8>,
    layout: Layout,
    len: usize,
    mr: NonNull<Mr>,
    pub(super) lkey: u32,
    pub(super) rkey: u32,
}

// SAFETY: plain memory; what touches it is raw copies of spans the
// transport's counters hand to one side at a time, and atomics.
unsafe impl Send for Region {}
// SAFETY: as above.
unsafe impl Sync for Region {}

impl Region {
    pub(super) fn new(dev: &Arc<Device>, len: usize, remote_write: bool) -> io::Result<Region> {
        let size = len.div_ceil(4096).max(1) * 4096;
        let layout = Layout::from_size_align(size, 4096)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a region too large"))?;
        // SAFETY: a layout of non-zero size.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let mut access = IBV_ACCESS_LOCAL_WRITE;
        if remote_write {
            access |= IBV_ACCESS_REMOTE_WRITE;
        }
        // SAFETY: the memory just allocated, `size` bytes, kept until the
        // registration is undone.
        let mr = unsafe { (dev.verbs.reg_mr)(dev.pd.as_ptr(), ptr.as_ptr().cast(), size, access) };
        let mr = match made(mr, "registering memory with the RDMA device") {
            Ok(mr) => mr,
            Err(e) => {
                // SAFETY: allocated above with this layout.
                unsafe { dealloc(ptr.as_ptr(), layout) };
                return Err(e);
            }
        };
        // SAFETY: the registration just made.
        let (lkey, rkey) = unsafe { ((*mr.as_ptr()).lkey, (*mr.as_ptr()).rkey) };
        Ok(Region {
            dev: Arc::clone(dev),
            ptr,
            layout,
            len,
            mr,
            lkey,
            rkey,
        })
    }

    /// The address a peer writes to.
    pub(super) fn addr(&self) -> u64 {
        self.ptr.as_ptr().addr() as u64
    }

    /// Copies `bytes` in at `at`.
    pub(super) fn copy_in(&self, at: usize, bytes: &[u8]) {
        assert!(at <= self.len && bytes.len() <= self.len - at);
        // SAFETY: inside the region; the span is this side's alone until
        // it posts it.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.as_ptr().add(at), bytes.len());
        }
    }

    /// Copies `out.len()` bytes out from `at`.
    pub(super) fn copy_out(&self, at: usize, out: &mut [u8]) {
        assert!(at <= self.len && out.len() <= self.len - at);
        // SAFETY: inside the region; the peer does not write the span
        // until this side tells it the span is read.
        unsafe {
            core::ptr::copy_nonoverlapping(self.ptr.as_ptr().add(at), out.as_mut_ptr(), out.len());
        }
    }

    /// The counter at `at`, which a peer writes.
    pub(super) fn word(&self, at: usize) -> &AtomicU32 {
        assert!(at.is_multiple_of(4) && at <= self.len && self.len - at >= 4);
        // SAFETY: 4-aligned (the region is page-aligned), inside it, and
        // alive as long as `self`; touched otherwise only by the device.
        unsafe { AtomicU32::from_ptr(self.ptr.as_ptr().add(at).cast()) }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: the registration and memory this region made, released
        // once, the registration first; the queue that posted from it is
        // destroyed before (field order in the transport).
        unsafe {
            (self.dev.verbs.dereg_mr)(self.mr.as_ptr());
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

/// A finished work request.
#[derive(Debug, Clone, Copy)]
pub(super) struct Completion {
    pub(super) wr_id: u64,
    pub(super) status: u32,
    pub(super) opcode: u32,
    pub(super) imm: Option<u32>,
}

/// A reliable connected queue pair, its two completion queues, and the
/// channel the receive queue's events arrive on.
pub(super) struct Queue {
    dev: Arc<Device>,
    channel: NonNull<CompChannel>,
    send_cq: NonNull<c_void>,
    recv_cq: NonNull<c_void>,
    qp: NonNull<Qp>,
    pub(super) qpn: u32,
}

// SAFETY: as `Device`'s; the receive queue is polled by the reader alone,
// the send queue under the writer's lock.
unsafe impl Send for Queue {}
// SAFETY: as above.
unsafe impl Sync for Queue {}

impl Queue {
    pub(super) fn new(dev: &Arc<Device>, sends: u32, recvs: u32) -> io::Result<Queue> {
        let v = &dev.verbs;
        let ctx = dev.ctx.as_ptr();
        // SAFETY: the open context, for each object below; each failure
        // releases what was made before it.
        unsafe {
            let channel = made(
                (v.create_comp_channel)(ctx),
                "creating a completion channel",
            )?;
            let send_cq = (v.create_cq)(ctx, sends as c_int, null_mut(), null_mut(), 0);
            let send_cq = match made(send_cq, "creating a completion queue") {
                Ok(cq) => cq,
                Err(e) => {
                    (v.destroy_comp_channel)(channel.as_ptr());
                    return Err(e);
                }
            };
            let recv_cq = (v.create_cq)(ctx, recvs as c_int, null_mut(), channel.as_ptr(), 0);
            let recv_cq = match made(recv_cq, "creating a completion queue") {
                Ok(cq) => cq,
                Err(e) => {
                    (v.destroy_cq)(send_cq.as_ptr());
                    (v.destroy_comp_channel)(channel.as_ptr());
                    return Err(e);
                }
            };
            let mut init = QpInitAttr {
                qp_context: null_mut(),
                send_cq: send_cq.as_ptr(),
                recv_cq: recv_cq.as_ptr(),
                srq: null_mut(),
                cap: QpCap {
                    max_send_wr: sends,
                    max_recv_wr: recvs,
                    max_send_sge: 1,
                    max_recv_sge: 1,
                    max_inline_data: INLINE,
                },
                qp_type: IBV_QPT_RC,
                sq_sig_all: 0,
            };
            let qp = match made(
                (v.create_qp)(dev.pd.as_ptr(), &mut init),
                "creating a queue pair",
            ) {
                Ok(qp) => qp,
                Err(e) => {
                    (v.destroy_cq)(recv_cq.as_ptr());
                    (v.destroy_cq)(send_cq.as_ptr());
                    (v.destroy_comp_channel)(channel.as_ptr());
                    return Err(e);
                }
            };
            Ok(Queue {
                dev: Arc::clone(dev),
                channel,
                send_cq,
                recv_cq,
                qpn: (*qp.as_ptr()).qp_num,
                qp,
            })
        }
    }

    fn modify(&self, attr: &mut QpAttr, mask: c_int, what: &str) -> io::Result<()> {
        // SAFETY: the live queue pair and an attribute struct of the
        // header's layout.
        status(
            unsafe { (self.dev.verbs.modify_qp)(self.qp.as_ptr(), attr, mask) },
            what,
        )
    }

    /// Reset to init: remote writes allowed.
    pub(super) fn to_init(&self) -> io::Result<()> {
        let mut a = QpAttr {
            qp_state: IBV_QPS_INIT,
            port_num: self.dev.port,
            qp_access_flags: (IBV_ACCESS_LOCAL_WRITE | IBV_ACCESS_REMOTE_WRITE) as c_uint,
            ..QpAttr::default()
        };
        self.modify(
            &mut a,
            IBV_QP_STATE | IBV_QP_PKEY_INDEX | IBV_QP_PORT | IBV_QP_ACCESS_FLAGS,
            "moving the queue pair to INIT",
        )
    }

    /// Init to ready-to-receive, towards the peer's queue pair.
    pub(super) fn to_rtr(&self, qpn: u32, psn: u32, gid: &[u8; 16], mtu: u8) -> io::Result<()> {
        let mut a = QpAttr {
            qp_state: IBV_QPS_RTR,
            path_mtu: u32::from(mtu),
            dest_qp_num: qpn,
            rq_psn: psn,
            max_dest_rd_atomic: 1,
            min_rnr_timer: 12,
            ah_attr: AhAttr {
                grh: GlobalRoute {
                    dgid: *gid,
                    sgid_index: self.dev.gid_index,
                    hop_limit: 64,
                    ..GlobalRoute::default()
                },
                is_global: 1,
                port_num: self.dev.port,
                ..AhAttr::default()
            },
            ..QpAttr::default()
        };
        self.modify(
            &mut a,
            IBV_QP_STATE
                | IBV_QP_AV
                | IBV_QP_PATH_MTU
                | IBV_QP_DEST_QPN
                | IBV_QP_RQ_PSN
                | IBV_QP_MAX_DEST_RD_ATOMIC
                | IBV_QP_MIN_RNR_TIMER,
            "moving the queue pair to RTR",
        )
    }

    /// Ready-to-receive to ready-to-send. A peer short of receives is
    /// retried without end (`rnr_retry` 7); a lost one, seven times.
    pub(super) fn to_rts(&self, psn: u32) -> io::Result<()> {
        let mut a = QpAttr {
            qp_state: IBV_QPS_RTS,
            sq_psn: psn,
            timeout: 14,
            retry_cnt: 7,
            rnr_retry: 7,
            max_rd_atomic: 1,
            ..QpAttr::default()
        };
        self.modify(
            &mut a,
            IBV_QP_STATE
                | IBV_QP_TIMEOUT
                | IBV_QP_RETRY_CNT
                | IBV_QP_RNR_RETRY
                | IBV_QP_SQ_PSN
                | IBV_QP_MAX_QP_RD_ATOMIC,
            "moving the queue pair to RTS",
        )
    }

    /// Posts a receive with no buffer: what a write with immediate data
    /// consumes.
    pub(super) fn post_recv(&self, wr_id: u64) -> io::Result<()> {
        let mut wr = RecvWr {
            wr_id,
            next: null_mut(),
            sg_list: null_mut(),
            num_sge: 0,
        };
        let mut bad = null_mut();
        // SAFETY: a request of the header's layout, read during the call.
        status(
            unsafe { (self.dev.post_recv)(self.qp.as_ptr().cast(), &mut wr, &mut bad) },
            "posting a receive",
        )
    }

    /// Posts a signalled write of `len` bytes of `src` from `at`, to
    /// `remote` (an address and key the peer handed over), carrying
    /// `imm` when given.
    pub(super) fn post_write(
        &self,
        wr_id: u64,
        src: &Region,
        at: usize,
        len: usize,
        remote: (u64, u32),
        imm: Option<u32>,
    ) -> io::Result<()> {
        assert!(at <= src.len && len <= src.len - at);
        let mut sge = Sge {
            addr: src.addr() + at as u64,
            length: u32::try_from(len)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a write over 4 GiB"))?,
            lkey: src.lkey,
        };
        self.post(wr_id, &mut sge, IBV_SEND_SIGNALED, remote, imm)
    }

    /// Posts a signalled write of a few bytes copied at the post.
    pub(super) fn post_write_inline(
        &self,
        wr_id: u64,
        bytes: &[u8],
        remote: (u64, u32),
    ) -> io::Result<()> {
        assert!(bytes.len() <= INLINE as usize);
        let mut sge = Sge {
            addr: bytes.as_ptr().addr() as u64,
            length: bytes.len() as u32,
            lkey: 0,
        };
        self.post(
            wr_id,
            &mut sge,
            IBV_SEND_SIGNALED | IBV_SEND_INLINE,
            remote,
            None,
        )
    }

    fn post(
        &self,
        wr_id: u64,
        sge: &mut Sge,
        flags: c_uint,
        (remote_addr, rkey): (u64, u32),
        imm: Option<u32>,
    ) -> io::Result<()> {
        let mut wr = SendWr {
            wr_id,
            next: null_mut(),
            sg_list: sge,
            num_sge: 1,
            opcode: if imm.is_some() {
                IBV_WR_RDMA_WRITE_WITH_IMM
            } else {
                IBV_WR_RDMA_WRITE
            },
            send_flags: flags,
            imm_data: imm.unwrap_or(0).to_be(),
            remote_addr,
            rkey,
            wr_rest: [0; 5],
            tail: [0; 7],
        };
        let mut bad = null_mut();
        // SAFETY: a request of the header's layout whose buffer is either
        // registered memory the caller holds (`src`) or copied at the post
        // (inline).
        status(
            unsafe { (self.dev.post_send)(self.qp.as_ptr().cast(), &mut wr, &mut bad) },
            "posting a write",
        )
    }

    fn poll(&self, cq: NonNull<c_void>, out: &mut Vec<Completion>) -> io::Result<()> {
        let mut wc = [Wc::default(); 16];
        // SAFETY: room for 16 completions, as asked.
        let n = unsafe { (self.dev.poll_cq)(cq.as_ptr(), wc.len() as c_int, wc.as_mut_ptr()) };
        if n < 0 {
            return Err(io::Error::other("polling a completion queue failed"));
        }
        out.extend(wc[..n as usize].iter().map(|w| Completion {
            wr_id: w.wr_id,
            status: w.status,
            opcode: w.opcode,
            imm: (w.wc_flags & IBV_WC_WITH_IMM != 0).then(|| u32::from_be(w.imm_data)),
        }));
        Ok(())
    }

    /// The send queue's finished requests, appended to `out`.
    pub(super) fn poll_send(&self, out: &mut Vec<Completion>) -> io::Result<()> {
        self.poll(self.send_cq, out)
    }

    /// The receive queue's, appended to `out`.
    pub(super) fn poll_recv(&self, out: &mut Vec<Completion>) -> io::Result<()> {
        self.poll(self.recv_cq, out)
    }

    /// Asks for an event on the channel at the receive queue's next
    /// completion.
    pub(super) fn arm_recv(&self) -> io::Result<()> {
        // SAFETY: the live receive queue.
        status(
            unsafe { (self.dev.req_notify_cq)(self.recv_cq.as_ptr(), 0) },
            "arming the completion queue",
        )
    }

    /// The channel's descriptor: readable when an event waits.
    pub(super) fn channel_fd(&self) -> RawFd {
        // SAFETY: the live channel; its descriptor does not change.
        unsafe { (*self.channel.as_ptr()).fd }
    }

    /// Takes and acknowledges the event waiting on the channel.
    pub(super) fn take_event(&self) -> io::Result<()> {
        let mut cq = null_mut();
        let mut ctx = null_mut();
        // SAFETY: the live channel; called when its descriptor is readable,
        // so it does not block.
        status(
            unsafe { (self.dev.verbs.get_cq_event)(self.channel.as_ptr(), &mut cq, &mut ctx) },
            "taking a completion event",
        )?;
        // SAFETY: acknowledges the one event just taken, on its queue, so
        // destroying the queue does not wait for it.
        unsafe { (self.dev.verbs.ack_cq_events)(cq, 1) };
        Ok(())
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        let v = &self.dev.verbs;
        // SAFETY: each object made in `new`, destroyed once, the queue
        // pair before the queues it completes on; every event was
        // acknowledged when taken.
        unsafe {
            (v.destroy_qp)(self.qp.as_ptr());
            (v.destroy_cq)(self.recv_cq.as_ptr());
            (v.destroy_cq)(self.send_cq.as_ptr());
            (v.destroy_comp_channel)(self.channel.as_ptr());
        }
    }
}

/// Waits until one of `fds` is readable: which ones are, in order (none,
/// when a signal cut the wait short), or `None` once `timeout` passes.
pub(super) fn readable(
    fds: [RawFd; 2],
    timeout: Option<Duration>,
) -> io::Result<Option<[bool; 2]>> {
    let mut polled = fds.map(|fd| libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    });
    let ms = timeout.map_or(-1, |t| {
        c_int::try_from(t.as_millis().max(1)).unwrap_or(c_int::MAX)
    });
    // SAFETY: two descriptors in a buffer of two.
    let n = unsafe { libc::poll(polled.as_mut_ptr(), 2, ms) };
    if n < 0 {
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            return Ok(Some([false; 2]));
        }
        return Err(e);
    }
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(polled.map(|p| p.revents != 0)))
}
