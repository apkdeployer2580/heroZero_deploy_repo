use core::pin::Pin;

use alloc::{borrow::ToOwned, boxed::Box, collections::BTreeMap, format, string::String, sync::Arc, vec, vec::Vec};

use jvm::{ClassInstance, Jvm, Result as JvmResult, runtime::JavaLangString};
use spin::Mutex;

use wie_backend::{Emulator, Event, JavaArg, JavaReceiver, Options, Platform, System, TaskRunner};
use wie_core_arm::{Allocator, ArmCore};
use wie_util::{Result, WieError};

use crate::{
    adf::{KtfAdf, find_client_bin},
    runtime::{JavaClassDefinition, JavaClassInstance, JavaValueExt, KtfJvmSupport, oom},
};

pub const IMAGE_BASE: u32 = 0x100000;

struct KtfTaskRunner {
    core: ArmCore,
}

#[async_trait::async_trait]
impl TaskRunner for KtfTaskRunner {
    async fn run(&self, future: Pin<Box<dyn Future<Output = Result<()>> + Send>>) -> Result<()> {
        self.core.run_in_thread(async move || future.await)?.await
    }
}

pub struct KtfEmulator {
    core: ArmCore,
    system: System,
    /// The JVM, published by `start` once it exists.
    ///
    /// ★ Needed by HOST-INITIATED guest calls. The JVM identifies a "thread" by the
    /// executor TASK id (`WieJavaRuntime::current_task_id`), and `Jvm::find_calling_class`
    /// does `threads.get(&thread_id).unwrap()`. A `System::spawn`ed task is a brand-new
    /// task id, so unless it attaches first, any JVM path the guest code reaches panics
    /// on an unregistered thread. `KtfWIPICContext::spawn` has always wrapped its callback
    /// in `attach_thread`/`detach_thread` for exactly this reason; `call_guest_function`
    /// did not, which is why it had never actually worked on a device.
    jvm: Arc<Mutex<Option<Jvm>>>,
}

impl KtfEmulator {
    pub fn from_archive(platform: Box<dyn Platform>, files: BTreeMap<String, Vec<u8>>, options: Options) -> Result<Self> {
        let adf = files
            .get("__adf__")
            .ok_or_else(|| WieError::FatalError("Missing __adf__ in KTF archive".into()))?;
        let adf = KtfAdf::parse(adf);

        tracing::info!("Loading app {}, pid {}, mclass {}", adf.aid, adf.pid, adf.mclass);

        let jar_filename = format!("{}.jar", adf.aid);

        Self::load(platform, &jar_filename, &adf.pid, &adf.aid, Some(adf.mclass), &files, options)
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
        files.contains_key("__adf__")
    }

    pub fn loadable_jar(jar: &[u8]) -> bool {
        find_client_bin(jar).is_ok()
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
        let system = System::new(platform, pid, aid, KtfTaskRunner { core: core.clone() });

        for (path, data) in files {
            let path = path.trim_start_matches("P/");
            system.filesystem().add_virtual(path, data.clone());
        }

        Allocator::init(&mut core)?;

        let mut core_clone = core.clone();
        let mut system_clone = system.clone();
        let jar_filename_clone = jar_filename.to_owned();
        let jvm_slot: Arc<Mutex<Option<Jvm>>> = Arc::new(Mutex::new(None));
        let jvm_slot_clone = jvm_slot.clone();

        // ★ 2026-10-21: a GUEST task — `park_guest` holds it with the game's other threads.
        system.spawn_guest(async move || Self::start(&mut core_clone, &mut system_clone, jar_filename_clone, main_class_name, jvm_slot_clone).await);

        Ok(Self { core, system, jvm: jvm_slot })
    }

    #[tracing::instrument(name = "start", skip_all)]
    async fn start(
        core: &mut ArmCore,
        system: &mut System,
        jar_filename: String,
        main_class_name: Option<String>,
        jvm_slot: Arc<Mutex<Option<Jvm>>>,
    ) -> Result<()> {
        let (jvm, class_loader) = KtfJvmSupport::init(core, system, Some(&jar_filename)).await?;
        // Publish it for host-initiated guest calls (see the `jvm` field).
        *jvm_slot.lock() = Some(jvm.clone());

        let main_class_name = if let Some(x) = main_class_name {
            x
        } else {
            return Err(WieError::FatalError("Main class not found".into()));
        };

        let main_class_name = main_class_name.replace('.', "/");

        let main_class_name_java = JavaLangString::from_rust_string(&jvm, &main_class_name).await.unwrap();
        let _main_class: Box<dyn ClassInstance> = jvm
            .invoke_virtual(
                &class_loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                (main_class_name_java.clone(),),
            )
            .await
            .unwrap();

        let mut args_array = jvm.instantiate_array("Ljava/lang/String;", 1).await.unwrap();
        jvm.store_array(&mut args_array, 0, vec![main_class_name_java]).await.unwrap();
        let result: JvmResult<()> = jvm
            .invoke_static("org/kwis/msp/lcdui/Main", "main", "([Ljava/lang/String;)V", (args_array,))
            .await;

        if let Err(x) = result {
            // `oom::to_wie_err` first: building the stack trace allocates, so an out-of-memory exception reaching here would panic inside
            // `JvmSupport::to_wie_err` instead of being reported.
            return Err(oom::to_wie_err(&jvm, x).await);
        }

        Ok(())
    }

    /// The same wrapper `JavaValue::from_raw` builds for a returned object (array or
    /// plain), so the pin set's equality finds it again on release — or `None` for a
    /// handle that cannot safely be wrapped.
    ///
    /// ⛔ The pre-flight is the point (2026-09-21, review). `JavaValue::from_raw` runs
    /// `instance.class().unwrap().is_array().unwrap()` for a `Class` type, and the second
    /// of those reads the class NAME through `ptr_class`. The only guard the callers have
    /// is `live_class_ptr`, whose own doc says it deliberately is NOT a name check
    /// "because on a handle that was never an instance `ptr_class` is a random word and
    /// reading a name through it would `unwrap` non-UTF-8" — and `liveness` documents the
    /// fields-pointer test as necessary, not sufficient (a freed slot handed back out
    /// passes it). The hosts of this call pass handles read out of guest tables validated
    /// by nothing but a heap-range test, so the `unwrap` was reachable, and it fires
    /// SYNCHRONOUSLY on the emulator worker: the thread dies while the UI and AudioTrack
    /// live on, which the player sees as a frozen screen with the music still playing.
    ///
    /// Asking `is_array()` here first turns that into a `None`. Nothing is lost by
    /// proving it twice: the class record is read again from the same memory on the same
    /// thread with no await in between, so `from_raw`'s unwraps cannot now fail.
    fn wrap_for_pin(&self, handle: u32, what: &str) -> Option<Box<dyn ClassInstance>> {
        let instance = JavaClassInstance::from_raw(handle, &self.core);
        let Some(ptr_class) = instance.live_class_ptr() else {
            tracing::warn!("{what} java object {handle:#x}: not a live instance — nothing to do");
            return None;
        };
        if JavaClassDefinition::from_raw(ptr_class, &self.core).is_array().is_err() {
            tracing::warn!("{what} java object {handle:#x}: its class record does not read — nothing to do");
            return None;
        }
        match jvm::JavaValue::from_raw(handle, &jvm::JavaType::Class("java/lang/Object".into()), &self.core) {
            jvm::JavaValue::Object(Some(obj)) => Some(obj),
            _ => None,
        }
    }
}

impl Emulator for KtfEmulator {
    fn handle_event(&mut self, event: Event) {
        self.system.event_queue().push(event)
    }

    fn next_wakeup_ms(&self) -> Option<u64> {
        self.system.next_wakeup_ms()
    }

    fn tick(&mut self) -> Result<()> {
        self.system.tick().map_err(|x| {
            let reg_stack = self.core.dump_reg_stack(IMAGE_BASE);
            match x {
                WieError::FatalError(msg) => WieError::FatalError(format!("{msg}\n{reg_stack}")),
                _ => WieError::FatalError(format!("{x}\n{reg_stack}")),
            }
        })
    }

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
        // Same as `call_guest_function`, plus the return value published for the host.
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
        // Spawn a cooperative task that runs the guest function to completion on
        // its own ARM thread context (fresh stack), the same mechanism WIPI
        // callbacks use (`KtfWIPICContext::call_function` -> `run_function`).
        // `run_function` saves/restores CPU context, so this never disturbs the
        // parked main-loop task; it executes on the next `System::tick`.
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

    fn call_java_method(&mut self, receiver: JavaReceiver<'_>, method: &str, descriptor: &str, args: &[JavaArg], out: alloc::sync::Arc<wie_backend::GuestCallResult>) {
        // Resolve and call through the JVM, not through the ARM core: on an
        // AOT-compiled guest the method body is not a plain `bl` target (see the
        // trait doc). `invoke_virtual` / `invoke_static` run exactly the dispatch the
        // guest itself would run.
        let jvm = self.jvm.lock().clone();
        let core = self.core.clone();
        let receiver = match receiver {
            JavaReceiver::Static { class, field, descriptor } => OwnedReceiver::Static(class.to_owned(), field.to_owned(), descriptor.to_owned()),
            JavaReceiver::Handle { handle, class } => OwnedReceiver::Handle(handle, class.to_owned()),
            JavaReceiver::Class(class) => OwnedReceiver::Class(class.to_owned()),
        };
        let (method, descriptor) = (method.to_owned(), descriptor.to_owned());
        let args: Vec<JavaArg> = args.to_vec();
        self.system.spawn(async move || {
            let what = format!("{}.{method}{descriptor}", receiver.class());
            let Some(jvm) = jvm.clone() else {
                tracing::warn!("java call {what}: no JVM yet");
                out.fail();
                return Ok(());
            };
            // The guard has to outlive the whole call: `invoke_virtual` reaches guest
            // code that reaches the JVM again, and that path indexes the thread map.
            let _attached = JvmThreadGuard::attach(Some(jvm.clone()));

            // ★ Every handle is checked BEFORE anything enters the guest (2026-09-21).
            // A handle is a heap address the host read some ticks ago; the collector
            // may have freed it since, and the bucket allocator may have handed the slot
            // out again as a different class. The JVM's own accessors `unwrap()` on a
            // freed record, and inside this spawned task that is a worker-killing panic
            // — the frozen-screen-with-music failure. So: a dead argument fails the
            // call, and a receiver whose live class is not the one the host named
            // fails the call. Neither reaches guest code.
            // `(the instance, its ptr_class)` if `handle` is a live record. The class is
            // compared as a POINTER against the class the host named, resolved by name
            // through the JVM — never read as a name off the handle, see `live_class_ptr`.
            let live = |handle: u32| live_instance(&core, handle);
            let values = match marshal_args(&core, &args) {
                Ok(v) => v,
                Err(i) => {
                    tracing::warn!("java call {what}: argument {i} ({:#x?}) is not a live java instance", args[i]);
                    out.fail();
                    return Ok(());
                }
            };

            if let OwnedReceiver::Class(class) | OwnedReceiver::Handle(_, class) = &receiver
                && let Err(e) = ensure_game_class(&jvm, class).await
            {
                tracing::warn!("java call {what}: class {class} could not be loaded: {e:?}");
                out.fail();
                return Ok(());
            }
            let result: JvmResult<jvm::JavaValue> = match receiver {
                OwnedReceiver::Class(class) => jvm.invoke_static(&class, &method, &descriptor, values).await,
                OwnedReceiver::Static(class, field, fdesc) => {
                    // ⛔ `Option<Box<dyn ClassInstance>>`, not `Box<dyn ClassInstance>` (2026-09-20,
                    // review): the non-optional conversion is `x.unwrap()`, so a static that is
                    // simply NULL — the singleton not constructed yet — would panic inside this
                    // spawned task instead of failing the call. That kills the emulator worker while
                    // the UI and the audio keep running, and `out` is never completed, so the caller
                    // waits forever on a frozen screen with the music still playing. The trait's
                    // contract is that this FAILS rather than leaving `out` pending.
                    let r: JvmResult<Option<Box<dyn ClassInstance>>> = jvm.get_static_field(&class, &field, &fdesc).await;
                    match r {
                        Ok(Some(r)) => jvm.invoke_virtual(&r, &method, &descriptor, values).await,
                        Ok(None) => {
                            tracing::warn!("java call {what}: {class}.{field}:{fdesc} is null — the singleton does not exist yet");
                            out.fail();
                            return Ok(());
                        }
                        Err(e) => {
                            tracing::warn!("java call {what}: {class}.{field}:{fdesc} unreadable: {e:?}");
                            out.fail();
                            return Ok(());
                        }
                    }
                }
                OwnedReceiver::Handle(handle, class) => {
                    // The class the host named, as the guest knows it: its `JavaClass`
                    // record. A class that does not resolve is a spelling mistake in the
                    // host's table and must not fall through to a call on the handle.
                    let expected = match jvm.resolve_class(&class).await {
                        Ok(c) => (*c.definition).as_any().downcast_ref::<crate::runtime::JavaClassDefinition>().map(|d| d.ptr_raw),
                        Err(e) => {
                            tracing::warn!("java call {what}: class {class} does not resolve: {e:?}");
                            out.fail();
                            return Ok(());
                        }
                    };
                    match (live(handle), expected) {
                        (Some((obj, ptr_class)), Some(want)) if ptr_class == want => jvm.invoke_virtual(&obj, &method, &descriptor, values).await,
                        // ★ 2026-10-16: a live instance of a SUBCLASS of the named class is a valid receiver too — the call
                        //   is virtual, so it runs the override if there is one (a boss `ca` for a `bd` method). The chain
                        //   is walked by the JVM's own subtyping (`is_instance`), never read as a name off the handle.
                        (Some((obj, _)), Some(_)) if jvm.is_instance(&*obj, &class) => jvm.invoke_virtual(&obj, &method, &descriptor, values).await,
                        (Some((_, ptr_class)), want) => {
                            tracing::warn!("java call {what}: receiver {handle:#x} is a live instance of class {ptr_class:#x}, not {class} ({want:#x?}) — refused");
                            out.fail();
                            return Ok(());
                        }
                        (None, _) => {
                            tracing::warn!("java call {what}: receiver {handle:#x} is not a live java instance — refused");
                            out.fail();
                            return Ok(());
                        }
                    }
                }
            };
            match result {
                Ok(v) => {
                    // The raw return word: a primitive's value, an object's handle, 0 for
                    // void. `as_raw` is the same marshalling the guest's own calls use;
                    // a two-word result is folded to its low word (nothing the host calls
                    // returns one, and a `J`/`D` handle would be a lie anyway).
                    let raw = match v {
                        jvm::JavaValue::Long(_) | jvm::JavaValue::Double(_) => v.as_raw64().0,
                        _ => v.as_raw(),
                    };
                    // ★ A returned object is pinned BEFORE the result is published — see the
                    // trait doc. This frame is its only reference, and the frame is about to
                    // go; the host releases it with `release_java_object`.
                    if let jvm::JavaValue::Object(Some(obj)) = &v {
                        jvm.pin_object(obj.clone());
                    }
                    tracing::debug!("java call {what} -> {raw:#x}");
                    out.ok(raw);
                }
                Err(e) => {
                    tracing::warn!("java call {what} failed: {e:?}");
                    out.fail();
                }
            }
            Ok(())
        });
    }

    fn new_java_object(&mut self, class: &str, init_descriptor: &str, args: &[JavaArg], out: alloc::sync::Arc<wie_backend::GuestCallResult>) {
        let jvm = self.jvm.lock().clone();
        let core = self.core.clone();
        let (class, init_descriptor) = (class.to_owned(), init_descriptor.to_owned());
        let args: Vec<JavaArg> = args.to_vec();
        self.system.spawn(async move || {
            let what = format!("new {class}{init_descriptor}");
            let Some(jvm) = jvm.clone() else {
                tracing::warn!("java call {what}: no JVM yet");
                out.fail();
                return Ok(());
            };
            let _attached = JvmThreadGuard::attach(Some(jvm.clone()));
            let values = match marshal_args(&core, &args) {
                Ok(v) => v,
                Err(i) => {
                    tracing::warn!("java call {what}: argument {i} ({:#x?}) is not a live java instance", args[i]);
                    out.fail();
                    return Ok(());
                }
            };
            if let Err(e) = ensure_game_class(&jvm, &class).await {
                tracing::warn!("java call {what}: class {class} could not be loaded: {e:?}");
                out.fail();
                return Ok(());
            }
            match jvm.new_class(&class, &init_descriptor, values).await {
                Ok(obj) => {
                    // Pinned like a returned object — `new_class` pushed it on THIS frame's
                    // locals, and this frame is gone the moment the task returns.
                    let raw = jvm::JavaValue::Object(Some(obj.clone())).as_raw();
                    jvm.pin_object(obj);
                    tracing::debug!("java call {what} -> {raw:#x}");
                    out.ok(raw);
                }
                Err(e) => {
                    tracing::warn!("java call {what} failed: {e:?}");
                    out.fail();
                }
            }
            Ok(())
        });
    }

    fn new_java_byte_array(&mut self, bytes: &[u8], out: alloc::sync::Arc<wie_backend::GuestCallResult>) {
        let jvm = self.jvm.lock().clone();
        let bytes: Vec<u8> = bytes.to_vec();
        self.system.spawn(async move || {
            let Some(jvm) = jvm.clone() else {
                tracing::warn!("java byte[]: no JVM yet");
                out.fail();
                return Ok(());
            };
            let _attached = JvmThreadGuard::attach(Some(jvm.clone()));
            let mut array = match jvm.instantiate_array("B", bytes.len()).await {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("java byte[{}]: could not allocate: {e:?}", bytes.len());
                    out.fail();
                    return Ok(());
                }
            };
            let written = match jvm.array_raw_buffer_mut(&mut array).await {
                Ok(mut buf) => buf.write(0, &bytes),
                Err(e) => Err(e),
            };
            if let Err(e) = written {
                tracing::warn!("java byte[{}]: could not fill: {e:?}", bytes.len());
                out.fail();
                return Ok(());
            }
            let raw = jvm::JavaValue::Object(Some(array.clone())).as_raw();
            jvm.pin_object(array);
            tracing::debug!("java byte[{}] -> {raw:#x}", bytes.len());
            out.ok(raw);
            Ok(())
        });
    }

    fn new_java_string_array(&mut self, values: &[&str], out: alloc::sync::Arc<wie_backend::GuestCallResult>) {
        let jvm = self.jvm.lock().clone();
        let values: Vec<String> = values.iter().map(|v| (*v).to_owned()).collect();
        self.system.spawn(async move || {
            let Some(jvm) = jvm.clone() else {
                tracing::warn!("java String[]: no JVM yet");
                out.fail();
                return Ok(());
            };
            let _attached = JvmThreadGuard::attach(Some(jvm.clone()));
            // Each string is pushed on THIS frame's locals by `instantiate_class`, so all of
            // them are roots until the array holds them; nothing runs a collection between.
            let mut strings: Vec<Box<dyn ClassInstance>> = Vec::with_capacity(values.len());
            for v in &values {
                match JavaLangString::from_rust_string(&jvm, v).await {
                    Ok(s) => strings.push(s),
                    Err(e) => {
                        tracing::warn!("java String[{}]: could not make {v:?}: {e:?}", values.len());
                        out.fail();
                        return Ok(());
                    }
                }
            }
            let mut array = match jvm.instantiate_array("Ljava/lang/String;", values.len()).await {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("java String[{}]: could not allocate: {e:?}", values.len());
                    out.fail();
                    return Ok(());
                }
            };
            if let Err(e) = jvm.store_array(&mut array, 0, strings).await {
                tracing::warn!("java String[{}]: could not fill: {e:?}", values.len());
                out.fail();
                return Ok(());
            }
            let raw = jvm::JavaValue::Object(Some(array.clone())).as_raw();
            jvm.pin_object(array);
            tracing::debug!("java String[{}] {values:?} -> {raw:#x}", values.len());
            out.ok(raw);
            Ok(())
        });
    }

    fn init_java_class(&mut self, class: &str, out: alloc::sync::Arc<wie_backend::GuestCallResult>) {
        let jvm = self.jvm.lock().clone();
        let class = class.to_owned();
        self.system.spawn(async move || {
            let Some(jvm) = jvm.clone() else {
                tracing::warn!("java init {class}: no JVM yet");
                out.fail();
                return Ok(());
            };
            let _attached = JvmThreadGuard::attach(Some(jvm.clone()));
            if let Err(e) = ensure_game_class(&jvm, &class).await {
                tracing::warn!("java init {class}: the class could not be loaded: {e:?}");
                out.fail();
                return Ok(());
            }
            let resolved = match jvm.resolve_class(&class).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("java init {class}: does not resolve: {e:?}");
                    out.fail();
                    return Ok(());
                }
            };
            match jvm.ensure_initialized(&resolved).await {
                Ok(()) => {
                    tracing::info!("java init {class}: initialized");
                    out.ok(0);
                }
                Err(e) => {
                    tracing::warn!("java init {class}: <clinit> failed: {e:?}");
                    out.fail();
                }
            }
            Ok(())
        });
    }

    fn park_guest(&mut self, on: bool) {
        if self.system.guest_parked() != on {
            tracing::info!("guest tasks {}", if on { "parked" } else { "released" });
        }
        self.system.park_guest(on);
    }

    fn alloc_guest(&mut self, len: u32) -> Option<u32> {
        match Allocator::alloc(&mut self.core, len) {
            Ok(a) => {
                tracing::info!("host block: {len} B of guest memory at {a:#x}");
                Some(a)
            }
            Err(e) => {
                tracing::warn!("host block: could not reserve {len} B of guest memory: {e:?}");
                None
            }
        }
    }

    fn pin_java_object(&mut self, handle: u32) -> bool {
        let Some(jvm) = self.jvm.lock().clone() else {
            return false;
        };
        let Some(obj) = self.wrap_for_pin(handle, "pin") else {
            return false;
        };
        jvm.pin_object(obj);
        tracing::debug!("pin java object {handle:#x}");
        true
    }

    fn release_java_object(&mut self, handle: u32) {
        let Some(jvm) = self.jvm.lock().clone() else {
            return;
        };
        // The pin set compares by concrete type (an array wrapper is not equal to the plain
        // instance wrapper on the same address), so rebuild the same wrapper `from_raw`
        // built when the call returned — and only for a record that is still live, since
        // `from_raw` reads the class through it.
        if let Some(obj) = self.wrap_for_pin(handle, "release") {
            jvm.unpin_object(&*obj);
            tracing::debug!("release java object {handle:#x}");
        }
    }
}

/// Make sure the JVM knows game class `name` before a host-initiated call names it.
///
/// ★ Why (2026-09-21): `Jvm::resolve_class` picks the class loader from the CALLING Java
/// frame, and a host-initiated call has none — "called outside of java" — so it falls
/// back to the system loader, which knows the runtime's own classes and nothing in
/// `client.bin`. A class the game has already used is in the registry and resolves; one
/// it has not (the inventory window `q` in a session that never opened INVENTORY) is
/// `NoClassDefFoundError`. The game's loader is a Java object with a static `instance`,
/// and asking it to `loadClass` is exactly what the guest's own class-load SVC ends up
/// doing, so this routes a host call through the same door.
async fn ensure_game_class(jvm: &Jvm, name: &str) -> JvmResult<()> {
    if jvm.has_class(name) {
        return Ok(());
    }
    let loader: Box<dyn ClassInstance> = jvm.get_static_field("net/wie/KtfClassLoader", "instance", "Lnet/wie/KtfClassLoader;").await?;
    let jname = JavaLangString::from_rust_string(jvm, name).await?;
    let _: Box<dyn ClassInstance> = jvm.invoke_virtual(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", (jname,)).await?;
    tracing::info!("java call: loaded class {name} through the game's class loader");
    Ok(())
}

/// `(the instance, its ptr_class)` if `handle` is a live record — the check every
/// host-supplied handle passes before it is handed to the JVM. See
/// `JavaClassInstance::live_class_ptr` for why the class is a pointer here, not a name.
fn live_instance(core: &ArmCore, handle: u32) -> Option<(Box<dyn ClassInstance>, u32)> {
    let instance = JavaClassInstance::from_raw(handle, core);
    let class = instance.live_class_ptr()?;
    Some((Box::new(instance) as Box<dyn ClassInstance>, class))
}

/// The host's arguments as JVM values, or the index of the first object handle that is
/// not a live instance (the call is refused before anything enters the guest).
fn marshal_args(core: &ArmCore, args: &[JavaArg]) -> core::result::Result<Vec<jvm::JavaValue>, usize> {
    let mut values: Vec<jvm::JavaValue> = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        values.push(match *a {
            JavaArg::Int(v) => jvm::JavaValue::Int(v),
            JavaArg::Object(0) => jvm::JavaValue::Object(None),
            JavaArg::Object(h) => match live_instance(core, h) {
                Some((obj, _)) => jvm::JavaValue::Object(Some(obj)),
                None => return Err(i),
            },
        });
    }
    Ok(values)
}

/// [`JavaReceiver`] with owned strings, for the spawned task.
enum OwnedReceiver {
    Static(String, String, String),
    Handle(u32, String),
    Class(String),
}

impl OwnedReceiver {
    fn class(&self) -> &str {
        match self {
            Self::Static(c, ..) | Self::Handle(_, c) | Self::Class(c) => c,
        }
    }
}

/// Registers the CURRENT executor task as a JVM thread for as long as it lives.
///
/// A `System::spawn`ed task gets a fresh task id, and the JVM keys its per-thread state
/// (the Java frame stack that `find_calling_class` reads) by exactly that id. Guest code
/// reached through a WIPI vtable can land in the JVM, and there it would index a thread
/// map that has no entry — `threads.get(&id).unwrap()`, i.e. a panic that kills the app.
///
/// `KtfWIPICContext::spawn` has always done this around its callbacks. Host-initiated
/// calls need it for the same reason, and a guard makes the detach unmissable on every
/// exit path (including the error paths, which is where the hand-written version of this
/// would have leaked a thread entry).
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
