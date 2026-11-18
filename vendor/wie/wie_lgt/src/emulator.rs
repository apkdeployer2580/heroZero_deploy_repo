use core::pin::Pin;

use alloc::{borrow::ToOwned, boxed::Box, collections::BTreeMap, format, string::String, sync::Arc, vec::Vec};

use jvm::{
    Jvm,
    runtime::{JavaIoInputStream, JavaLangClassLoader},
};
use spin::Mutex;

use wie_backend::{Emulator, Event, Options, Platform, System, TaskRunner, extract_zip};
use wie_core_arm::{Allocator, ArmCore};
use wie_jvm_support::{JvmSupport, RustJavaJvmImplementation};
use wie_util::{Result, WieError};

use crate::runtime::init::load_native;

struct LgtTaskRunner {
    core: ArmCore,
}

#[async_trait::async_trait]
impl TaskRunner for LgtTaskRunner {
    async fn run(&self, future: Pin<Box<dyn Future<Output = Result<()>> + Send>>) -> Result<()> {
        self.core.run_in_thread(async move || future.await)?.await
    }
}

pub struct LgtEmulator {
    core: ArmCore,
    system: System,
    /// Published by `do_start` once the JVM exists, for host-initiated guest calls.
    /// Same shape and reason as `KtfEmulator::jvm`: a host-driven call runs on a fresh
    /// task whose thread the JVM has never seen, and the first JVM touch inside the
    /// callee (`find_calling_class` → `threads.get(&id).unwrap()`) is a panic, not an
    /// error. Measured on LGT 2026-08-11: `writeSaveRecord`'s WIPI DB write dies there
    /// without the guard — the earlier "no AOT Java on the ARM side, no guard needed"
    /// note was wrong about the DB path.
    jvm: Arc<Mutex<Option<Jvm>>>,
}

impl LgtEmulator {
    pub fn from_archive(platform: Box<dyn Platform>, files: BTreeMap<String, Vec<u8>>, options: Options) -> Result<Self> {
        let app_info = files
            .get("app_info")
            .ok_or_else(|| WieError::FatalError("Missing app_info in LGT archive".into()))?;
        let app_info = LgtAppInfo::parse(app_info);

        tracing::info!("Loading app {}, pid {}, mclass {}", app_info.aid, app_info.pid, app_info.mclass);

        let jar_filename = format!("{}.jar", app_info.aid);

        Self::load(
            platform,
            &jar_filename,
            &app_info.pid,
            &app_info.aid,
            Some(app_info.mclass),
            &files,
            options,
        )
    }

    pub fn from_jar(
        platform: Box<dyn Platform>,
        jar_filename: &str,
        jar: Vec<u8>,
        pid: &str,
        aid: &str,
        main_class_name: Option<String>,
        options: Options,
    ) -> Result<Self> {
        let files = [(jar_filename.to_owned(), jar)].into_iter().collect();

        Self::load(platform, jar_filename, pid, aid, main_class_name, &files, options)
    }

    pub fn loadable_archive(files: &BTreeMap<String, Vec<u8>>) -> bool {
        files.contains_key("app_info")
    }

    pub fn loadable_jar(jar: &[u8]) -> bool {
        let Ok(files) = extract_zip(jar) else {
            return false;
        };

        files.contains_key("binary.mod")
    }

    fn load(
        platform: Box<dyn Platform>,
        jar_filename: &str,
        pid: &str,
        aid: &str,
        main_class_name: Option<String>,
        files: &BTreeMap<String, Vec<u8>>,
        mut options: Options,
    ) -> Result<Self> {
        let mut core = ArmCore::new(options.enable_gdbserver, options.profile.take())?;
        let system = System::new(platform, pid, aid, LgtTaskRunner { core: core.clone() });

        for (filename, data) in files {
            let filename = filename.trim_start_matches("P/");
            system.filesystem().add_virtual(filename, data.clone())
        }

        Allocator::init(&mut core)?;

        let main_class_name = main_class_name.map(|x| x.replace('.', "/"));

        let mut core_clone = core.clone();
        let mut system_clone = system.clone();
        let main_class_name_clone = main_class_name.clone();
        let jar_filename = jar_filename.to_owned();
        let jvm_slot: Arc<Mutex<Option<Jvm>>> = Arc::new(Mutex::new(None));
        let jvm_slot_clone = jvm_slot.clone();

        system.spawn(async move || Self::do_start(&mut core_clone, &mut system_clone, jar_filename, main_class_name_clone, jvm_slot_clone).await);

        Ok(Self { core, system, jvm: jvm_slot })
    }

    #[tracing::instrument(name = "start", skip_all)]
    async fn do_start(
        core: &mut ArmCore,
        system: &mut System,
        jar_filename: String,
        _main_class_name: Option<String>,
        jvm_slot: Arc<Mutex<Option<Jvm>>>,
    ) -> Result<()> {
        let protos = [wie_midp::get_protos().into(), wie_wipi_java::get_protos().into()];
        let jvm = JvmSupport::new_jvm(system, Some(&jar_filename), Box::new(protos), &[], RustJavaJvmImplementation).await?; // TODO use lgt's java implementation

        // Publish it for host-initiated guest calls (see the `jvm` field).
        *jvm_slot.lock() = Some(jvm.clone());

        let class_loader = jvm.current_class_loader().await.unwrap();
        let stream = JavaLangClassLoader::get_resource_as_stream(&jvm, &class_loader, "binary.mod")
            .await
            .unwrap()
            .unwrap();

        let binary_mod = JavaIoInputStream::read_until_end(&jvm, &stream).await.unwrap();

        load_native(core, system, &jvm, &binary_mod).await?;

        Ok(())
    }
}

impl Emulator for LgtEmulator {
    fn handle_event(&mut self, event: Event) {
        self.system.event_queue().push(event)
    }

    fn next_wakeup_ms(&self) -> Option<u64> {
        self.system.next_wakeup_ms()
    }

    fn tick(&mut self) -> Result<()> {
        self.system.tick().map_err(|x| {
            let reg_stack = self.core.dump_reg_stack(0x1000); // TODO: hardcode
            match x {
                WieError::FatalError(msg) => WieError::FatalError(format!("{msg}\n{reg_stack}")),
                _ => WieError::FatalError(format!("{x}\n{reg_stack}")),
            }
        })
    }

    // ★ wie fork: host introspection parity with KtfEmulator, for the Android
    // overlay HUD / host-driven guest calls. LGT clet events already run through
    // `core.run_function` on their own cooperative task (see clet_wrapper_card.rs),
    // so host-initiated calls use the exact same mechanism.
    //
    // ★★ The JVM thread guard IS needed here too (2026-08-11). The earlier note said
    // "no AOT Java on the ARM side to attach to" — true, but the WIPI *DB* path a
    // guest callee may take (writeSaveRecord → database::*) touches the JVM from the
    // calling thread, and an unattached thread id panics in `find_calling_class`
    // (`threads.get(&id).unwrap()`). Measured with the host-driven save chain: the
    // serialise step completed and the first record write killed the emulator.
    fn read_memory(&self, address: u32, len: usize) -> Vec<u8> {
        use wie_util::ByteRead;
        let mut buf = alloc::vec![0u8; len];
        match self.core.read_bytes(address, &mut buf) {
            Ok(n) => {
                buf.truncate(n);
                buf
            }
            Err(_) => Vec::new(),
        }
    }

    fn write_memory(&mut self, address: u32, data: &[u8]) -> bool {
        use wie_util::ByteWrite;
        self.core.write_bytes(address, data).is_ok()
    }

    fn call_guest_function_out(&mut self, address: u32, args: &[u32], out: alloc::sync::Arc<wie_backend::GuestCallResult>) {
        let mut core = self.core.clone();
        let args: Vec<u32> = args.to_vec();
        let jvm = self.jvm.lock().clone();
        self.system.spawn(async move || {
            let _attached = JvmThreadGuard::attach(jvm);
            match core.run_function::<u32>(address, &args).await {
                Ok(ret) => {
                    tracing::debug!("guest call {address:#x} -> {ret:#x}");
                    out.ok(ret);
                }
                Err(e) => {
                    tracing::warn!("guest call {address:#x} failed: {e:?}");
                    out.fail();
                }
            }
            Ok(())
        });
    }

    fn call_guest_function(&mut self, address: u32, args: &[u32]) {
        let mut core = self.core.clone();
        let args: Vec<u32> = args.to_vec();
        let jvm = self.jvm.lock().clone();
        self.system.spawn(async move || {
            let _attached = JvmThreadGuard::attach(jvm);
            match core.run_function::<u32>(address, &args).await {
                Ok(ret) => tracing::debug!("guest call {address:#x} -> {ret:#x}"),
                Err(e) => tracing::warn!("guest call {address:#x} failed: {e:?}"),
            }
            Ok(())
        });
    }
}

/// Attach the calling thread to the JVM for the duration of a host-initiated guest
/// call, detaching on every exit path. Same rationale (and shape) as the KTF guard:
/// the callee may touch the JVM, and the JVM resolves the CURRENT THREAD against a
/// registration map — an unregistered id is `threads.get(&id).unwrap()`, a panic.
struct JvmThreadGuard(Option<Jvm>);

impl JvmThreadGuard {
    fn attach(jvm: Option<Jvm>) -> Self {
        // `None` = the JVM does not exist yet (a call issued during boot). Running the
        // guest function anyway is right: not every target touches the JVM, and refusing
        // would be a silent no-op.
        if let Some(jvm) = &jvm
            && let Err(e) = jvm.attach_thread()
        {
            tracing::warn!("could not attach a JVM thread for a host-initiated guest call: {e:?}");
            return Self(None);
        }
        Self(jvm)
    }
}

impl Drop for JvmThreadGuard {
    fn drop(&mut self) {
        if let Some(jvm) = &self.0
            && let Err(e) = jvm.detach_thread()
        {
            tracing::warn!("could not detach the JVM thread after a host-initiated guest call: {e:?}");
        }
    }
}

// almost similar to KtfAdf.. can we merge these?
struct LgtAppInfo {
    aid: String,
    pid: String,
    mclass: String,
}

impl LgtAppInfo {
    pub fn parse(data: &[u8]) -> Self {
        let mut aid = String::new();
        let mut pid = String::new();
        let mut mclass = String::new();

        let mut lines = data.split(|x| *x == b'\n');

        for line in &mut lines {
            if line.starts_with(b"AID:") {
                aid = String::from_utf8_lossy(&line[4..]).into();
            } else if line.starts_with(b"PID:") {
                pid = String::from_utf8_lossy(&line[4..]).into();
            } else if line.starts_with(b"MClass:") {
                mclass = String::from_utf8_lossy(&line[7..]).into();
            }
            // TODO load name, it's in euc-kr..
        }

        Self { aid, pid, mclass }
    }
}
