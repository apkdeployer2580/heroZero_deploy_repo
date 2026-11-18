use alloc::boxed::Box;

use crate::{audio_sink::AudioSink, database::DatabaseRepository, screen::Screen, time::Instant};

pub trait Platform: Send + Sync {
    fn screen(&self) -> &dyn Screen;
    fn now(&self) -> Instant;
    fn database_repository(&self) -> &dyn DatabaseRepository;
    fn filesystem(&self) -> &dyn Filesystem;
    fn audio_sink(&self) -> Box<dyn AudioSink>;
    fn write_stdout(&self, buf: &[u8]);
    fn write_stderr(&self, buf: &[u8]);
    fn exit(&self);
    fn vibrate(&self, duration_ms: u64, intensity: u8);

    /// Host TCP sockets for the WIPI net API, or `None` on platforms with no
    /// networking.
    ///
    /// Defaulted deliberately: `Platform` has several implementors (wie_cli,
    /// wie_web, test_utils, in-crate test stubs) and only the Android host
    /// implements this, so a required method would break them all for no gain.
    /// `None` makes every net slot fail cleanly rather than hang the guest.
    fn network(&self) -> Option<&dyn Network> {
        None
    }
}

/// Host networking behind the WIPI C net API.
///
/// The contract mirrors what the guest expects (see `TRACK_C_PRIVATE_SERVER.md`
/// §3) rather than BSD sockets, because the guest's error codes are load-bearing
/// and translating them twice invites mistakes.
///
/// Deliberately **not** `async`: it must stay object-safe in a `no_std` crate,
/// and every operation is non-blocking anyway, so there is nothing to await.
/// Connect is the one slow operation and is handled by starting it with
/// [`Network::connect`] and polling [`Network::poll_connect`].
pub trait Network: Send + Sync {
    /// Allocate a socket. `af`/`ty` follow BSD (`2` = AF_INET, `1` = SOCK_STREAM).
    /// Returns a descriptor, or negative on failure.
    fn socket(&self, af: i32, ty: i32) -> i32;

    /// Begin connecting. `addr` is the four octets in memory order (as produced
    /// by the guest's own `inet_addr`), `port_be` is already in network order.
    /// `name` is the guest's channel label, e.g. `"BILLSOCK"` — informational.
    ///
    /// Returns 0 if the attempt started; completion is observed via
    /// [`Network::poll_connect`].
    fn connect(&self, fd: i32, addr: u32, port_be: u16, name: &str) -> i32;

    /// `0` = still connecting, `1` = connected, negative = failed.
    fn poll_connect(&self, fd: i32) -> i32;

    /// Bytes accepted, or negative on error. May be a partial write.
    fn send(&self, fd: i32, data: &[u8]) -> i32;

    /// `n > 0` = bytes read, `-19` = nothing available (socket still healthy),
    /// `0` = peer closed, other negative = error.
    ///
    /// Implementations must never conflate "no data yet" with "peer gone": the
    /// guest tears the connection down on anything that is not positive or -19.
    fn recv(&self, fd: i32, buf: &mut [u8]) -> i32;

    /// Register interest in readability (guest called `MC_netSetReadCB`).
    ///
    /// Returns `true` if this call created the registration, `false` if one was
    /// already outstanding for `fd`. The caller uses that to spawn exactly one
    /// waiter per arm — the guest re-arms after every `recv` that returns -19,
    /// so without this an idle connection would accumulate waiters forever.
    fn arm_read(&self, fd: i32) -> bool;

    /// `1` = data is available (or the peer is gone, which the guest also needs
    /// to learn), `0` = nothing yet, `-1` = socket is dead/unknown.
    ///
    /// Returning 1 or -1 clears the registration made by [`Network::arm_read`].
    fn poll_readable(&self, fd: i32) -> i32;

    fn close(&self, fd: i32);

    /// Drop every socket (guest called `MC_netClose`).
    fn close_all(&self);
}

/// Platform filesystem abstraction. Every method is scoped by `aid`;
/// implementations MUST NOT cross aid boundaries.
#[async_trait::async_trait]
pub trait Filesystem: Send + Sync {
    async fn exists(&self, aid: &str, path: &str) -> bool;

    async fn size(&self, aid: &str, path: &str) -> Option<usize>;

    /// Read up to `count` bytes starting at `offset` into `buf[..count]`.
    ///
    /// - File missing → `None`.
    /// - `offset >= size` (read past EOF) → `Some(0)`.
    /// - Otherwise → `Some(n)` where `0 < n <= count`. Short reads allowed
    ///   at end of file.
    /// - Caller guarantees `buf.len() >= count`. Implementations only write
    ///   to `buf[..n]`.
    async fn read(&self, aid: &str, path: &str, offset: usize, count: usize, buf: &mut [u8]) -> Option<usize>;

    /// Write `data` starting at `offset`.
    ///
    /// - Creates the file (and any missing intermediate directories) if it
    ///   does not yet exist. A zero-length `data` is a valid way to
    ///   materialize an empty file.
    /// - If `offset + data.len() > current_size` the implementation MUST
    ///   automatically extend the file, zero-filling the gap.
    /// - Returns the number of bytes actually written. On success this
    ///   equals `data.len()`.
    /// - On failure (path rejected, disk full, permission denied, etc.)
    ///   MUST return `0` and log via `tracing::warn!` or `tracing::error!`.
    ///   Silent `0` returns are forbidden.
    async fn write(&self, aid: &str, path: &str, offset: usize, data: &[u8]) -> usize;

    /// Truncate the file to exactly `len` bytes. Creates the file if
    /// missing.
    /// - `len > current_size` → zero-fill extend.
    /// - `len < current_size` → tail bytes dropped.
    async fn truncate(&self, aid: &str, path: &str, len: usize);
}
