use alloc::{boxed::Box, format, string::String, vec, vec::Vec};

use wipi_types::wipic::WIPICWord;

use wie_util::{Result, WieError, read_null_terminated_string_bytes};

use crate::{WIPICMethodBody, WIPICResult, context::WIPICContext, method::MethodBody};

/// Track C (private server) recon instrumentation.
///
/// The Hero3 network features (복구 = server-side slot backup, 네트워크 창고,
/// 캐시템, 오브 결합) talk to Hanbit servers that no longer exist. Before the
/// real socket layer can be built we need ground truth on *which* WIPI net
/// slots the game calls and with what arguments — the WIPI C net API signatures
/// are not authoritatively documented anywhere we can rely on.
///
/// `trace_stub` replaces the `Unimplemented` net stubs (which abort the
/// emulator with FatalError) with a logging no-op that returns a chosen value,
/// so the game keeps running and the whole call sequence lands in logcat.
///
/// Guest pointers are peeked so string args (the hardcoded server IPs live at
/// image offsets 0xab1fc / 0xab218 = guest 0x1ab1fc / 0x1ab218) and packet
/// buffers show up inline.
const PEEK_LEN: usize = 64;

/// `wie_core_arm::core::HEAP_BASE`, duplicated so this crate stays core-agnostic.
const HEAP_BASE: WIPICWord = 0x4000_0000;

fn peek(context: &dyn WIPICContext, addr: WIPICWord) -> Option<String> {
    // Only image (0x100000..) and heap (0x40000000..) addresses are worth a read;
    // everything else is a small int / flag and a read would just be noise.
    let plausible = (0x100000..0x400000).contains(&addr) || (HEAP_BASE..HEAP_BASE + 0x800000).contains(&addr);
    if !plausible {
        return None;
    }

    let mut buf = [0u8; PEEK_LEN];
    let read = context.read_bytes(addr, &mut buf).ok()?;
    let buf = &buf[..read];

    let hex = buf.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join("");
    let ascii = buf
        .iter()
        .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' })
        .collect::<String>();

    Some(format!("[{hex}|{ascii}]"))
}

/// Logging no-op for a not-yet-implemented net slot. Returns `ret` verbatim.
///
/// Implemented as a `MethodBody` rather than the usual closure because the
/// closure form can't hold the `&mut dyn WIPICContext` across the returned
/// future (invariance on the trait object) and we need the context to peek
/// pointer arguments.
pub fn trace_stub(id: WIPICWord, name: &'static str, ret: i32) -> WIPICMethodBody {
    struct TraceStub {
        id: WIPICWord,
        name: &'static str,
        ret: i32,
    }

    #[async_trait::async_trait]
    impl MethodBody<WieError> for TraceStub {
        async fn call(&self, context: &mut dyn WIPICContext, args: Box<[WIPICWord]>) -> Result<WIPICResult> {
            // The KTF dispatcher always hands over 9 words (r0..r3 + stack);
            // the first 6 are plenty for every net slot.
            let shown = &args[..args.len().min(6)];

            let arglist = shown.iter().map(|a| format!("{a:#x}")).collect::<Vec<_>>().join(", ");
            let peeks = shown
                .iter()
                .enumerate()
                .filter_map(|(i, &a)| peek(context, a).map(|p| format!(" a{i}={p}")))
                .collect::<String>();

            tracing::warn!("NETTRACE slot{} {}({}) -> {}{}", self.id, self.name, arglist, self.ret, peeks);

            Ok(WIPICResult {
                results: vec![self.ret as WIPICWord],
            })
        }
    }

    Box::new(TraceStub { id, name, ret })
}

pub async fn connect(context: &mut dyn WIPICContext, cb: WIPICWord, param: WIPICWord) -> Result<i32> {
    tracing::warn!("NETTRACE slot0 MC_netConnect(cb={cb:#x}, param={param:#x}) -> 0, callback pending");

    struct ConnectCallback {
        cb: WIPICWord,
        param: WIPICWord,
    }

    #[async_trait::async_trait]
    impl MethodBody<WieError> for ConnectCallback {
        #[tracing::instrument(name = "timer", skip_all)]
        async fn call(&self, context: &mut dyn WIPICContext, _: Box<[WIPICWord]>) -> Result<WIPICResult> {
            context.system().sleep(1).await; // simulate some delay

            // Track C recon: report SUCCESS (0) instead of the historical
            // M_E_ERROR (-1). With an error the game never advances past
            // "network bring-up" and no further net slot is ever exercised,
            // which is exactly what hid the call sequence until now. Reporting
            // success lets it walk into the socket calls so `trace_stub` can
            // record them.
            tracing::warn!("NETTRACE slot0 MC_netConnect callback(result=0 SUCCESS, param={:#x})", self.param);
            context.call_function(self.cb, &[0, self.param]).await?;

            Ok(WIPICResult { results: Vec::new() })
        }
    }

    context.spawn(Box::new(ConnectCallback { cb, param }))?;

    Ok(0)
}

/// KTF net slot 30 — connect a *named* socket. VERIFIED against Hero3.
///
/// ```text
/// int slot30(int fd, u32 addr_be, s16 port_be, const char *name,
///            void (*cb)(int param, int status), int param)
/// ```
///
/// `addr_be` is the raw 4 bytes produced by the game's own inet_addr
/// (image+0x7e5ac), i.e. octets in memory order — little-endian on ARM, so
/// `0x1c12ded2` reads as 210.222.18.28. `port_be` is already `MC_utilHtons`ed.
/// `name` is a channel label; Hero3 opens "BILLSOCK" first.
///
/// The return value is only checked for `< 0` (with -19 tolerated); the real
/// outcome is delivered asynchronously to `cb`, which treats `status == 0` as
/// success (image+0x7e4b0) and anything else as fatal (tears the socket down
/// via net slot 33 + slot 1).
///
/// Opens a real host socket via `Platform::network()` and delivers the outcome
/// to the guest callback. A platform without networking fails immediately
/// *through the callback* rather than by return value — returning an error here
/// but never calling back would leave the game on "서버 응답 기다리는중" forever.
pub async fn connect_named(
    context: &mut dyn WIPICContext,
    fd: i32,
    addr: WIPICWord,
    port: WIPICWord,
    name: WIPICWord,
    cb: WIPICWord,
    param: WIPICWord,
) -> Result<i32> {
    let octets = addr.to_le_bytes();
    let port_be = (port & 0xffff) as u16;
    // `name == 0` is the LGT shape (see `connect_lgt`), which has no channel label at
    // all — do not try to read a string out of guest address 0 for it.
    let label = if name == 0 {
        String::new()
    } else {
        read_null_terminated_string_bytes(context, name)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    };

    tracing::warn!(
        "NETTRACE slot30 connect_named(fd={fd}, {}.{}.{}.{}:{} name={label:?}, cb={cb:#x}, param={param:#x})",
        octets[0],
        octets[1],
        octets[2],
        octets[3],
        u16::from_be(port_be),
    );

    let system = context.system().clone();
    let started = match system.platform().network() {
        Some(net) => net.connect(fd, addr, port_be, &label) >= 0,
        None => {
            tracing::error!("NETTRACE slot30: platform has no network backend");
            false
        }
    };

    struct ConnectedCallback {
        cb: WIPICWord,
        param: WIPICWord,
        fd: i32,
        started: bool,
    }

    #[async_trait::async_trait]
    impl MethodBody<WieError> for ConnectedCallback {
        async fn call(&self, context: &mut dyn WIPICContext, _: Box<[WIPICWord]>) -> Result<WIPICResult> {
            // Poll the host connect until it resolves. Bounded so a black-holed
            // route cannot leave this task (and the guest's UI) pending forever;
            // the host applies its own connect timeout as well.
            const POLL_MS: u64 = 50;
            const MAX_POLLS: u32 = 240; // 12s

            let mut status = -1i32;
            if self.started {
                let system = context.system().clone();
                for _ in 0..MAX_POLLS {
                    context.system().sleep(POLL_MS).await;
                    match system.platform().network().map(|n| n.poll_connect(self.fd)) {
                        Some(0) => continue,
                        Some(1) => {
                            status = 0;
                            break;
                        }
                        _ => break,
                    }
                }
            }

            if status == 0 {
                tracing::warn!("NETTRACE slot30 connect callback(param={:#x}, status=0 SUCCESS)", self.param);
            } else {
                tracing::warn!("NETTRACE slot30 connect callback(param={:#x}, status={status} FAILED)", self.param);
            }
            context.call_function(self.cb, &[self.param, status as WIPICWord]).await?;

            Ok(WIPICResult { results: Vec::new() })
        }
    }

    context.spawn(Box::new(ConnectedCallback { cb, param, fd, started }))?;

    Ok(0)
}

/// KTF net slot 2 — create a socket. Hero3 only ever asks for (2, 1).
/// LGT net slot 3 — `MC_netSocketConnect(fd, addr, port, cb, param)`.
///
/// ── WHY LGT NEEDS ITS OWN ARM (MIGRATION_LGT.md §2.18) ───────────────────────
/// KTF connects through slot **30** (`connect_named`, six arguments including a channel
/// label — Hero3 opens `"BILLSOCK"` first). LGT connects through slot **3** with five,
/// and no label. Device trace, with slot 3 still a logging stub:
///
/// ```text
/// slot0   MC_netConnect(cb=0x18901)       -> 0, callback pending
/// slot2   socket(af=2, type=1)            -> fd 3
/// slot901 (0x1f9b=8091, 1, 0x14005e4, 0, cb=0x18901, ctx=0x150ea80)
/// slot904 (0x5c734, 1, 0x14005e4, 0x14005f4, cb=0x18901, ctx=0x150ea80)
/// slot3   (fd=3, 0, 0, 0x18b35, 0xf, …)   -> 0        <- the connect
/// slot0   MC_netConnect callback(result=0 SUCCESS)
///         …retried on fd 4, then slot6 close(4), slot1 MC_netClose, gave up
/// ```
/// `0x18b35` carries the Thumb bit and sits beside the confirmed `MC_netConnect`
/// callback `0x18901`, so it is the completion callback; `0xf` is its param. The trailing
/// word the stub printed is a stale register — the dispatcher hands every body nine
/// arguments whether the function takes them or not.
///
/// **`addr` and `port` arrive as 0** because the destination was supplied by slots
/// 901/904 beforehand. That costs nothing here: `Platform::network()`'s Android
/// implementation ignores the requested address entirely and redirects every connect to
/// the configured private server (`network.rs`, `REDIRECT_ENABLED`) — the original
/// Hanbit hosts are long gone. So the only thing this arm has to do is what a connect
/// does: open the socket and fire the callback. Delegating to [`connect_named`] with an
/// empty label keeps ONE implementation of the polling/timeout/callback contract.
pub async fn connect_lgt(context: &mut dyn WIPICContext, fd: i32, addr: WIPICWord, port: WIPICWord, cb: WIPICWord, param: WIPICWord) -> Result<i32> {
    tracing::warn!("NETTRACE slot3 connect_lgt(fd={fd}, addr={addr:#x}, port={port:#x}, cb={cb:#x}, param={param:#x})");
    connect_named(context, fd, addr, port, 0, cb, param).await
}

/// LGT WIPI id **901** — `htons`. Static RE of the connect callback at binary.mod
/// `0x18900` (MIGRATION_LGT.md §2.18-(1')): the caller passes ONE argument (the
/// port — 8091 primary, 0x8de secondary), truncates the return to u16
/// (`lsls #16; lsrs #16`) and hands it to slot 3 `connect_lgt` as `port`.
/// `connect_named` reads that argument as big-endian (`u16::from_be`), which is
/// exactly what a htons result is. The other four words the dispatcher shows in
/// traces are stale caller registers, not arguments.
///
/// While this was a `trace_stub` returning 0, slot 3 received `port=0` — harmless
/// only because the Android backend redirects every connect to the configured
/// private server. Implementing it makes the guest-visible values truthful.
pub async fn htons(_context: &mut dyn WIPICContext, value: WIPICWord) -> Result<i32> {
    let swapped = ((value & 0xffff) as u16).swap_bytes();
    tracing::warn!("NETTRACE id901 htons({value:#x}) -> {swapped:#x}");
    Ok(swapped as i32)
}

/// LGT WIPI id **904** — resolve a host string to an address word (`inet_addr`).
/// Same RE as [`htons`]: the single real argument is a NUL-terminated host string
/// (`"210.222.18.28"` at `.text 0x5c734`, `"210.222.18.8"` at `0x5c744`), and the
/// return value goes verbatim into slot 3 `connect_lgt`'s `addr`, which
/// `connect_named` decodes with `addr.to_le_bytes()` = first octet in the low
/// byte — network byte order in a little-endian word, i.e. `inet_addr`.
///
/// Hero3's hosts are always dotted quads, so a parser is the whole job. A
/// non-quad string would need real DNS, which the backend doesn't expose — log
/// and return 0 (the previous stub behaviour) so the redirect still saves us.
pub async fn inet_addr(context: &mut dyn WIPICContext, host: WIPICWord) -> Result<i32> {
    let text = read_null_terminated_string_bytes(context, host)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();

    let mut octets = [0u8; 4];
    let mut count = 0;
    for (i, part) in text.split('.').enumerate() {
        if i >= 4 {
            count = 0;
            break;
        }
        match part.parse::<u8>() {
            Ok(v) => {
                octets[i] = v;
                count = i + 1;
            }
            Err(_) => {
                count = 0;
                break;
            }
        }
    }

    if count != 4 {
        tracing::warn!("NETTRACE id904 inet_addr({text:?}) -> 0 (not a dotted quad)");
        return Ok(0);
    }

    let addr = u32::from_le_bytes(octets);
    tracing::warn!("NETTRACE id904 inet_addr({text:?}) -> {addr:#x}");
    Ok(addr as i32)
}

pub async fn socket(context: &mut dyn WIPICContext, af: i32, ty: i32) -> Result<i32> {
    let system = context.system().clone();
    let fd = match system.platform().network() {
        Some(net) => net.socket(af, ty),
        None => {
            tracing::error!("NETTRACE slot2: platform has no network backend");
            -1
        }
    };

    tracing::warn!("NETTRACE slot2 socket(af={af}, type={ty}) -> {fd}");

    Ok(fd)
}

/// KTF net slot 13 — `MC_netSetReadCB`. VERIFIED against Hero3.
///
/// ```text
/// int slot13(int fd, void (*cb)(int param, int status), int param)
/// ```
///
/// This is load-bearing, not optional. The receive pump at image+0x7e63c calls
/// `recv`; on -19 it arms this callback and **stops polling**. If the callback
/// never fires the client never calls `recv` again, so server data sits unread
/// forever while the client happily heartbeats — observed exactly that way
/// before this was implemented.
///
/// Returns 0 (or -19) for OK; anything else makes the guest print
/// `readSocket() : CB SET ERROR %d` and tear the connection down.
pub async fn set_read_cb(context: &mut dyn WIPICContext, fd: i32, cb: WIPICWord, param: WIPICWord) -> Result<i32> {
    tracing::warn!("NETTRACE slot13 set_read_cb(fd={fd}, cb={cb:#x}, param={param:#x}) -> 0");

    let system = context.system().clone();
    let Some(net) = system.platform().network() else {
        tracing::error!("NETTRACE slot13: platform has no network backend");
        return Ok(-1);
    };

    // One waiter per arm. The guest re-arms after every -19, so without this
    // guard an idle connection accumulates a waiter every poll.
    if !net.arm_read(fd) {
        return Ok(0);
    }

    struct ReadWaiter {
        cb: WIPICWord,
        param: WIPICWord,
        fd: i32,
    }

    #[async_trait::async_trait]
    impl MethodBody<WieError> for ReadWaiter {
        async fn call(&self, context: &mut dyn WIPICContext, _: Box<[WIPICWord]>) -> Result<WIPICResult> {
            // Poll BEFORE sleeping, with a short first interval. The old
            // sleep-then-poll order at a flat 20ms taxed every server reply with
            // a full quantum even when the data was already waiting — and the
            // cash-shop/warehouse screens pay it four times per page turn (the
            // client replays `04 3d`/`05 3d`/`05 14`/`05 3f` on every page), a
            // wall-clock floor no in-game speed setting can touch. Backing off
            // 2→20ms keeps the steady-state cost identical to the old waiter;
            // after ~5s of silence (the guest's own net timeout scale) settle at
            // 100ms — by then the server is not answering (§2.23) and cheap
            // peeks are only needed so the fd still fires when it recovers.
            const POLL_MS_MAX: u64 = 20;
            const IDLE_AFTER_POLLS: u32 = 256; // ≈5s at the 20ms cadence
            const IDLE_POLL_MS: u64 = 100;

            let system = context.system().clone();
            let mut delay: u64 = 2;
            let mut polls: u32 = 0;
            loop {
                match system.platform().network().map(|n| n.poll_readable(self.fd)) {
                    Some(0) => {}
                    // 1 = data (or EOF, which the guest must also learn about).
                    Some(1) => break,
                    // Socket gone: drop the waiter silently. Firing the callback
                    // would send the guest into a recv on a dead fd, and its
                    // teardown path already runs on the close that killed it.
                    _ => return Ok(WIPICResult { results: Vec::new() }),
                }
                polls = polls.saturating_add(1);
                let ms = if polls >= IDLE_AFTER_POLLS { IDLE_POLL_MS } else { delay };
                context.system().sleep(ms).await;
                delay = (delay * 2).min(POLL_MS_MAX);
            }

            tracing::warn!("NETTRACE slot13 read callback(param={:#x}, status=0 READABLE)", self.param);
            context.call_function(self.cb, &[self.param, 0]).await?;

            Ok(WIPICResult { results: Vec::new() })
        }
    }

    context.spawn(Box::new(ReadWaiter { cb, param, fd }))?;

    Ok(0)
}

/// KTF net slot 33 — close one socket.
pub async fn socket_close_fd(context: &mut dyn WIPICContext, fd: i32) -> Result<i32> {
    tracing::warn!("NETTRACE slot33 close(fd={fd})");

    let system = context.system().clone();
    if let Some(net) = system.platform().network() {
        net.close(fd);
    }

    Ok(0)
}

/// KTF net slot 31 — send on a connected socket. VERIFIED against Hero3.
///
/// ```text
/// int slot31(int fd, const void *buf, int len)
/// ```
///
/// Returns the number of bytes accepted; a negative return makes the game
/// tear the connection down (slot 33 + slot 1), which is how the recon pass
/// first identified those two slots.
///
/// The payload is logged verbatim before transmission — this is the primary
/// instrument for the protocol work, and logging on the way out (rather than
/// inferring from the server side) separates "client never sent it" from
/// "server never saw it".
pub async fn socket_send(context: &mut dyn WIPICContext, fd: i32, buf: WIPICWord, len: i32) -> Result<i32> {
    let n = len.max(0) as usize;
    let mut data = vec![0u8; n];
    let read = context.read_bytes(buf, &mut data).unwrap_or(0);
    let data = &data[..read];
    let hex = data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");

    let system = context.system().clone();
    let sent = match system.platform().network() {
        Some(net) => net.send(fd, data),
        None => {
            tracing::error!("NETTRACE slot31: platform has no network backend");
            -1
        }
    };

    tracing::warn!("NETTRACE slot31 socket_send(fd={fd}, len={len}) -> {sent}  payload: {hex}");

    Ok(sent)
}

/// `M_E_WOULDBLOCK`-equivalent. Hero3 checks for exactly -19 in three places
/// (connect, setReadCB, recv) and treats it as "pending, not an error"; any
/// other negative — and, for recv, zero — means the peer is gone and the game
/// tears the connection down.
pub const M_E_WOULDBLOCK: i32 = -19;

/// KTF net slot 32 — receive on a connected socket. VERIFIED against Hero3.
///
/// ```text
/// int slot32(int fd, void *buf, int max)
/// ```
///
/// Return contract, decoded from the receive pump at image+0x7e63c:
///   * `n > 0`  — n bytes appended to the game's 1 KiB reassembly buffer
///   * `-19`    — nothing available; the game arms the read callback (slot 13)
///                and returns to its timer loop
///   * `<= 0` otherwise — treated as a dead peer -> full teardown
///
/// Framing above this layer: every message begins with a little-endian u32
/// total length that *includes* the 4-byte header (the game waits for >3 bytes,
/// reads the length from offset 0, then waits for that many bytes).
///
/// ⚠ The guest asks for 8 bytes more than its own receive object can hold (see
/// `AndroidNetwork::recv`); the host backend clamps delivery, so `max` is passed
/// through unmodified here and the clamp lives in exactly one place.
pub async fn socket_recv(context: &mut dyn WIPICContext, fd: i32, buf: WIPICWord, max: i32) -> Result<i32> {
    let cap = max.max(0) as usize;
    // ★ NO HEAP ALLOCATION ON THE POLL PATH (2026-08-13). The guest's server-wait loop
    // calls this tens of thousands of times a second and every one of those polls used to
    // allocate and zero a fresh 1 KiB `Vec` before finding out there was nothing to read.
    // Hero3 always asks for 1024 (`recv(fd, obj+12+used, 1024-used)`), so the stack
    // buffer is the only path taken in practice; the heap arm just keeps the function
    // honest for any `max` above it.
    const STACK_CAP: usize = 1024;
    let mut stack = [0u8; STACK_CAP];
    let mut heap: Vec<u8>;
    let data: &mut [u8] = if cap <= STACK_CAP {
        &mut stack[..cap]
    } else {
        heap = vec![0u8; cap];
        &mut heap
    };

    let system = context.system().clone();
    let n = match system.platform().network() {
        Some(net) => net.recv(fd, data),
        None => {
            tracing::error!("NETTRACE slot32: platform has no network backend");
            -1
        }
    };

    if n > 0 {
        let n_usize = (n as usize).min(cap);
        context.write_bytes(buf, &data[..n_usize])?;
        let hex = data[..n_usize].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
        tracing::warn!("NETTRACE slot32 socket_recv(fd={fd}, max={max}) -> {n}  payload: {hex}");
    } else if n != M_E_WOULDBLOCK {
        // 0 = peer closed, negative = error. Both make the guest tear down, so
        // they are worth seeing; -19 is the steady-state idle case and would
        // otherwise flood the log every poll.
        tracing::warn!("NETTRACE slot32 socket_recv(fd={fd}, max={max}) -> {n} (peer gone / error)");
    }

    Ok(n)
}

pub async fn close(context: &mut dyn WIPICContext) -> Result<()> {
    tracing::warn!("NETTRACE slot1 MC_netClose()");

    let system = context.system().clone();
    if let Some(net) = system.platform().network() {
        net.close_all();
    }

    Ok(())
}

pub async fn socket_close(_context: &mut dyn WIPICContext, fd: i32) -> Result<i32> {
    // Recon pass: report success so a teardown path doesn't itself look like
    // the failure. Was -1 (M_E_ERROR).
    tracing::warn!("NETTRACE slot6 MC_netSocketClose({fd}) -> 0");

    Ok(0)
}
