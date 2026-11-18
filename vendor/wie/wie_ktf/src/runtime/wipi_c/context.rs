use alloc::{borrow::ToOwned, boxed::Box, collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use jvm::{
    Jvm,
    runtime::{JavaIoInputStream, JavaLangClassLoader},
};
use spin::Mutex;
use wipi_types::wipic::{WIPICIndirectPtr, WIPICWord};

use wie_backend::{AsyncCallable, Event, Instant, System};
use wie_core_arm::{Allocator, ArmCore};
use wie_util::{ByteRead, ByteWrite, Result, read_generic, write_generic};
use wie_wipi_c::{WIPICContext, WIPICMethodBody};

#[derive(Clone)]
pub struct KtfWIPICContext {
    core: ArmCore,
    system: System,
    jvm: Jvm, // We need jvm to access resource in jvm. TODO is there better way to do this?
}

impl KtfWIPICContext {
    pub fn new(core: ArmCore, system: System, jvm: Jvm) -> Self {
        Self { core, system, jvm }
    }

    /// One jar resource, fetched at most once per process. `None` = no such resource
    /// (and that answer is cached too, so a name the game probes repeatedly does not
    /// keep rebuilding the archive to be told "no" again).
    async fn resource(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        let key = (self.system.aid().to_owned(), name.to_owned());
        if let Some(hit) = resource_cached(&key) {
            return hit;
        }

        let class_loader = self.jvm.current_class_loader().await.unwrap();
        let stream = JavaLangClassLoader::get_resource_as_stream(&self.jvm, &class_loader, name).await.unwrap();
        let value = match stream {
            None => None,
            Some(stream) => Some(Arc::new(JavaIoInputStream::read_until_end(&self.jvm, &stream).await.unwrap())),
        };

        resource_store(key, value.clone());
        value
    }
}

// ── MC_knlGetResource cache ──────────────────────────────────────────────────
//
// ★ 2026-08-09, and it is the largest single item in the "크리티컬이 뜨면 프레임이
// 떨어진다" chain — bigger than the SMAF parse, and it is paid by EVERY sound, not
// only the critical one.
//
// The guest re-fetches a sound from the jar on every play: `playSound` (guest
// 0x198244) -> 0x199a9c -> `MC_knlGetResourceID` (size) + `MC_knlGetResource`
// (bytes). Both land on [`get_resource_size`] / [`read_resource`] below, and both
// went through `JavaLangClassLoader::get_resource_as_stream` — which in RustJava
// builds a whole `ZipArchive` from a FRESH COPY of the entire jar, three times per
// call (findResource, openStream, getInputStream). Hero3's jar is 1,877,743 bytes
// and lives in the GUEST HEAP, so each copy is ~29 chunked reads through the page
// table, and each archive re-parses a 1,274-entry central directory.
//
// Six of those per sound. Measured at 3.9 ms on a desktop x86 — inside a frame that
// budgets 24 ms for interpretation, on a phone core several times slower. A crit
// pays it twice (its own `sd004_mf` plus whatever else that frame plays).
//
// Jar resources are immutable, so one fetch per name is enough for the process. The
// cache is a module global rather than a field because a fresh `KtfWIPICContext` is
// constructed for EVERY WIPI-C call (`wipi_c.rs:73`), so an instance field would
// never see a second hit. Keyed by `(aid, name)` so two emulators in one process
// (the workspace test binary) cannot read each other's resources.
//
// Bounded by [`RESOURCE_CACHE_MAX`]: past it, nothing new is cached and the old
// fetch-every-time path is used, so the worst case is exactly today's behaviour.
type ResourceKey = (String, String);
static RESOURCE_CACHE: Mutex<Option<BTreeMap<ResourceKey, Option<Arc<Vec<u8>>>>>> = Mutex::new(None);
static RESOURCE_CACHE_BYTES: Mutex<usize> = Mutex::new(0);

/// Cache ceiling. Hero3's whole uncompressed jar is 6.1 MB over 1,273 entries, so this
/// holds every resource the game can ask for and still refuses to grow without bound
/// for an archive that is larger than we have measured.
const RESOURCE_CACHE_MAX: usize = 12 * 1024 * 1024;

/// `Some(entry)` on a hit — the inner `Option` distinguishes "cached: this resource
/// does not exist" from "cached: here are its bytes", so a miss is not re-fetched
/// either. `None` means we have never looked.
fn resource_cached(key: &ResourceKey) -> Option<Option<Arc<Vec<u8>>>> {
    RESOURCE_CACHE.lock().as_ref().and_then(|m| m.get(key).cloned())
}

fn resource_store(key: ResourceKey, value: Option<Arc<Vec<u8>>>) {
    let len = value.as_ref().map(|v| v.len()).unwrap_or(0);
    let mut bytes = RESOURCE_CACHE_BYTES.lock();
    if *bytes + len > RESOURCE_CACHE_MAX {
        return;
    }
    *bytes += len;
    RESOURCE_CACHE.lock().get_or_insert_with(BTreeMap::new).insert(key, value);
}

#[async_trait::async_trait]
impl WIPICContext for KtfWIPICContext {
    fn alloc_raw(&mut self, size: WIPICWord) -> Result<WIPICWord> {
        Allocator::alloc(&mut self.core, size)
    }

    fn alloc(&mut self, size: WIPICWord) -> Result<WIPICIndirectPtr> {
        let ptr = Allocator::alloc(&mut self.core, size + 12)?; // all allocation has indirect pointer
        write_generic(&mut self.core, ptr, ptr + 4)?;
        write_generic(&mut self.core, ptr + 4, size)?;

        Ok(WIPICIndirectPtr(ptr))
    }

    fn free(&mut self, memory: WIPICIndirectPtr) -> Result<()> {
        let size: u32 = read_generic(&self.core, memory.0 + 4)?;
        Allocator::free(&mut self.core, memory.0, size + 12)?;

        Ok(())
    }

    fn free_raw(&mut self, address: WIPICWord, size: WIPICWord) -> Result<()> {
        Allocator::free(&mut self.core, address, size)?;

        Ok(())
    }

    fn data_ptr(&self, memory: WIPICIndirectPtr) -> Result<WIPICWord> {
        let base: WIPICWord = read_generic(&self.core, memory.0)?;

        Ok(base + 8) // all data has offset of 8 bytes
    }

    fn system(&mut self) -> &mut System {
        &mut self.system
    }

    async fn call_function(&mut self, address: WIPICWord, args: &[WIPICWord]) -> Result<WIPICWord> {
        self.core.run_function(address, args).await
    }

    fn spawn(&mut self, callback: WIPICMethodBody) -> Result<()> {
        struct SpawnProxy {
            context: KtfWIPICContext,
            callback: WIPICMethodBody,
        }

        impl AsyncCallable<Result<()>> for SpawnProxy {
            async fn call(mut self) -> Result<()> {
                self.context.jvm.attach_thread().unwrap();
                self.callback.call(&mut self.context, Box::new([])).await?;
                self.context.jvm.detach_thread().unwrap();

                Ok(())
            }
        }

        self.system.spawn_guest(SpawnProxy {
            context: self.clone(),
            callback,
        });

        Ok(())
    }

    async fn get_resource_size(&self, name: &str) -> Result<Option<usize>> {
        // Answered from the cache, so the SIZE query costs nothing and the following
        // read is already warm. See [`RESOURCE_CACHE`].
        Ok(self.resource(name).await.map(|v| v.len()))
    }

    async fn read_resource(&self, name: &str) -> Result<Vec<u8>> {
        // The old body `unwrap()`ed a missing resource into a panic. Preserved as far
        // as callers can tell — every caller of this asks `get_resource_size` first —
        // but an empty Vec is a better failure than taking the process down.
        Ok(self.resource(name).await.map(|v| v.as_ref().clone()).unwrap_or_default())
    }

    fn set_timer(&mut self, ptr_timer: WIPICWord, due: Instant, callback: WIPICMethodBody) {
        let context = self.clone();

        // Keyed by the guest's timer object: re-arming REPLACES the pending shot, and
        // MC_knlUnsetTimer can cancel it. Without this the queue accumulated one live
        // timer per screen the guest ever visited (PLAY_FEEDBACK_FIXES.md §41 item 7).
        let key = self.system().event_queue().arm_timer(ptr_timer as u64);
        self.system().event_queue().push(Event::timer_keyed(due, key, move || {
            let mut context = context.clone();

            async move {
                callback.call(&mut context, Box::new([])).await?;
                Ok(())
            }
        }))
    }

    fn unset_timer(&mut self, ptr_timer: WIPICWord) {
        self.system().event_queue().cancel_timer(ptr_timer as u64);
    }
}

impl ByteRead for KtfWIPICContext {
    fn read_bytes(&self, address: WIPICWord, result: &mut [u8]) -> wie_util::Result<usize> {
        self.core.read_bytes(address, result)
    }
}

impl ByteWrite for KtfWIPICContext {
    fn write_bytes(&mut self, address: WIPICWord, data: &[u8]) -> wie_util::Result<()> {
        self.core.write_bytes(address, data)
    }
}
