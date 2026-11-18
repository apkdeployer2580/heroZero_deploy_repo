use alloc::{boxed::Box, vec};
use core::time::Duration;

use java_class_proto::{JavaFieldProto, JavaMethodProto};
use java_constants::MethodAccessFlags;
use jvm::{ClassInstance, ClassInstanceRef, Jvm, Result, runtime::JavaLangString};

use crate::{RuntimeClassProto, RuntimeContext, SpawnCallback, classes::java::lang::Runnable};

// class java.lang.Thread
pub struct Thread;

impl Thread {
    pub fn as_proto() -> RuntimeClassProto {
        RuntimeClassProto {
            name: "java/lang/Thread",
            parent_class: Some("java/lang/Object"),
            interfaces: vec![],
            methods: vec![
                JavaMethodProto::new("<init>", "()V", Self::init, Default::default()),
                JavaMethodProto::new("<init>", "(Ljava/lang/Runnable;)V", Self::init_with_runnable, Default::default()),
                JavaMethodProto::new("start", "()V", Self::start, Default::default()),
                JavaMethodProto::new("join", "()V", Self::join, Default::default()),
                JavaMethodProto::new("run", "()V", Self::run, Default::default()),
                JavaMethodProto::new("isAlive", "()Z", Self::is_alive, Default::default()),
                JavaMethodProto::new("sleep", "(J)V", Self::sleep, MethodAccessFlags::NATIVE | MethodAccessFlags::STATIC),
                JavaMethodProto::new("yield", "()V", Self::r#yield, MethodAccessFlags::NATIVE | MethodAccessFlags::STATIC),
                JavaMethodProto::new("setPriority", "(I)V", Self::set_priority, Default::default()),
                // ★ heroZero fork (2026-10-21): 영웅서기 Zero cancels its HTTP worker with it.
                JavaMethodProto::new("interrupt", "()V", Self::interrupt, Default::default()),
                JavaMethodProto::new(
                    "currentThread",
                    "()Ljava/lang/Thread;",
                    Self::current_thread,
                    MethodAccessFlags::NATIVE | MethodAccessFlags::STATIC,
                ),
                // rustjava internal
                JavaMethodProto::new("<init>", "(Z)V", Self::init_internal, Default::default()),
            ],
            fields: vec![
                JavaFieldProto::new("id", "J", Default::default()),
                JavaFieldProto::new("target", "Ljava/lang/Runnable;", Default::default()),
                JavaFieldProto::new("alive", "Z", Default::default()),
                JavaFieldProto::new("interrupted", "Z", Default::default()),
            ],
            access_flags: Default::default(),
        }
    }

    async fn init(jvm: &Jvm, _: &mut RuntimeContext, this: ClassInstanceRef<Self>) -> Result<()> {
        tracing::debug!("java.lang.Thread::<init>({this:?})");

        let _: () = jvm.invoke_special(&this, "java/lang/Object", "<init>", "()V", ()).await?;

        Ok(())
    }

    async fn init_with_runnable(
        jvm: &Jvm,
        _: &mut RuntimeContext,
        mut this: ClassInstanceRef<Self>,
        target: ClassInstanceRef<Runnable>,
    ) -> Result<()> {
        tracing::debug!("java.lang.Thread::<init>({this:?}, {target:?})");

        let _: () = jvm.invoke_special(&this, "java/lang/Object", "<init>", "()V", ()).await?;

        jvm.put_field(&mut this, "target", "Ljava/lang/Runnable;", target).await?;

        Ok(())
    }

    async fn init_internal(jvm: &Jvm, context: &mut RuntimeContext, mut this: ClassInstanceRef<Self>, internal: bool) -> Result<()> {
        tracing::debug!("java.lang.Thread::<init>({this:?}, {internal:?})");

        let id = context.current_task_id();
        jvm.put_field(&mut this, "id", "J", id as i64).await?;

        Ok(())
    }

    async fn start(jvm: &Jvm, context: &mut RuntimeContext, mut this: ClassInstanceRef<Self>) -> Result<()> {
        tracing::debug!("java.lang.Thread::start({this:?})");

        struct ThreadStartProxy {
            jvm: Jvm,
            thread_id: i32,
            this: ClassInstanceRef<Thread>,
        }

        #[async_trait::async_trait]
        impl SpawnCallback for ThreadStartProxy {
            #[tracing::instrument(name = "java thread", fields(id = self.thread_id), skip_all)]
            async fn call(&self) -> Result<()> {
                tracing::trace!("Thread start");

                // Releases the pin `start` took, on EVERY exit - and `call` has several, one of
                // them a bare `?` on the thread body's own result.
                struct PinGuard(Jvm, Box<dyn ClassInstance>);

                impl Drop for PinGuard {
                    fn drop(&mut self) {
                        self.0.unpin_object(&*self.1);
                    }
                }

                let _pin = PinGuard(self.jvm.clone(), self.this.clone().into());

                self.jvm.attach_thread()?;

                let result: Result<()> = self.jvm.invoke_virtual(&self.this, "run", "()V", []).await;

                if let Err(jvm::JavaError::JavaException(x)) = result {
                    let string_writer = self.jvm.new_class("java/io/StringWriter", "()V", ()).await.unwrap();
                    let print_writer = self
                        .jvm
                        .new_class("java/io/PrintWriter", "(Ljava/io/Writer;)V", (string_writer.clone(),))
                        .await
                        .unwrap();

                    let _: () = self
                        .jvm
                        .invoke_virtual(&x, "printStackTrace", "(Ljava/io/PrintWriter;)V", (print_writer,))
                        .await
                        .unwrap();

                    let trace = self
                        .jvm
                        .invoke_virtual(&string_writer, "toString", "()Ljava/lang/String;", [])
                        .await
                        .unwrap();

                    tracing::error!(
                        "Uncaught exception in thread {}:\n{}",
                        self.thread_id,
                        JavaLangString::to_rust_string(&self.jvm, &trace).await.unwrap()
                    );
                } else {
                    result?;
                }

                // Cleanup must run BEFORE detach_thread: put_field can throw (a guest that
                // manages java objects natively may free its own Thread instance the moment
                // run() returns), and constructing that exception's message string requires
                // this thread to still be registered.
                let mut this = self.this.clone();
                if let Err(x) = self.jvm.put_field(&mut this, "alive", "Z", false).await {
                    tracing::warn!("Thread {} exit: could not clear alive flag (instance freed by guest?): {:?}", self.thread_id, x);
                }
                self.jvm.object_notify(&self.this, usize::MAX);

                self.jvm.detach_thread()?;

                Ok(())
            }
        }

        jvm.put_field(&mut this, "alive", "Z", true).await?;

        let id: i32 = jvm.invoke_virtual(&this, "hashCode", "()I", ()).await?;

        // ★ Pinned HERE, not inside the callback, because the gap this closes opens the moment
        // `start` returns: from then until the spawned task runs its first instruction, the only
        // reference to this Thread lives in the `ThreadStartProxy` below - a Rust struct, which is
        // not something the collector scans. Measured on device 2026-09-10: the KTF backend freed
        // the instance in that window and `Thread::run` then read `target` out of a corpse.
        // Released at the end of `call`, on every exit path.
        jvm.pin_object(this.clone().into());

        context.spawn(
            jvm,
            Box::new(ThreadStartProxy {
                jvm: jvm.clone(),
                thread_id: id,
                this: this.clone(),
            }),
        );

        Ok(())
    }

    async fn run(jvm: &Jvm, _: &mut RuntimeContext, this: ClassInstanceRef<Self>) -> Result<()> {
        tracing::debug!("java.lang.Thread::run({this:?})");

        let target: ClassInstanceRef<Runnable> = jvm.get_field(&this, "target", "Ljava/lang/Runnable;").await?;
        if !target.is_null() {
            let _: () = jvm.invoke_virtual(&target, "run", "()V", ()).await?;
        }

        Ok(())
    }

    async fn join(jvm: &Jvm, _context: &mut RuntimeContext, this: ClassInstanceRef<Self>) -> Result<()> {
        tracing::debug!("java.lang.Thread::join({this:?})");

        loop {
            let listener = jvm.object_listen(&this);
            let alive: bool = jvm.get_field(&this, "alive", "Z").await?;
            if !alive {
                return Ok(());
            }
            listener.await;
        }
    }

    async fn is_alive(jvm: &Jvm, _: &mut RuntimeContext, this: ClassInstanceRef<Self>) -> Result<bool> {
        tracing::debug!("java.lang.Thread::isAlive({this:?})");
        let alive: bool = jvm.get_field(&this, "alive", "Z").await?;
        Ok(alive)
    }

    async fn sleep(_: &Jvm, context: &mut RuntimeContext, duration: i64) -> Result<()> {
        tracing::debug!("java.lang.Thread::sleep({duration:?})");

        context.sleep(Duration::from_millis(duration as _)).await;

        Ok(())
    }

    async fn r#yield(_: &Jvm, context: &mut RuntimeContext) -> Result<()> {
        tracing::debug!("java.lang.Thread::yield()");
        context.r#yield().await;

        Ok(())
    }

    async fn set_priority(_: &Jvm, _: &mut RuntimeContext, this: ClassInstanceRef<Thread>, new_priority: i32) -> Result<()> {
        tracing::warn!("stub java.lang.Thread::setPriority({this:?}, {new_priority:?})");

        Ok(())
    }

    /// Sets the interrupt flag and nothing more: a thread blocked in `sleep`/`wait`/`join`
    /// is NOT woken with an InterruptedException (the scheduler has no hook for that).
    /// Enough for the one caller this was added for — 영웅서기 Zero's HTTP worker, whose
    /// connection already fails at once because the platform is offline by design.
    async fn interrupt(jvm: &Jvm, _: &mut RuntimeContext, mut this: ClassInstanceRef<Self>) -> Result<()> {
        tracing::debug!("java.lang.Thread::interrupt({this:?})");

        jvm.put_field(&mut this, "interrupted", "Z", true).await
    }

    async fn current_thread(jvm: &Jvm, _: &mut RuntimeContext) -> Result<ClassInstanceRef<Self>> {
        tracing::warn!("stub java.lang.Thread::currentThread()");

        let thread = jvm.new_class("java/lang/Thread", "(Z)V", (true,)).await?;

        Ok(thread.into())
    }
}
