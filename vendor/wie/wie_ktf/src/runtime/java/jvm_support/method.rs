use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::{
    fmt::{self, Debug, Formatter},
    mem::size_of,
    ops::{Deref, DerefMut},
};
use futures::TryFutureExt;

use java_class_proto::JavaMethodProto;
use java_constants::MethodAccessFlags;
use jvm::{ClassInstance, JavaError, JavaType, JavaValue, Jvm, Method, Result as JvmResult};
use wipi_types::ktf::java::{
    JavaExceptionHandler as RawJavaExceptionHandler, JavaMethodDefinition as RawJavaMethod,
    JavaMethodExceptionTableEntry as RawJavaMethodExceptionTableEntry,
};

use alloc::sync::Arc;
use wie_core_arm::{
    Allocator, ArmCore, EmulatedFunction, EmulatedFunctionParam, RUN_FUNCTION_LR, RegisteredFunction, RegisteredFunctionHolder, ResultWriter,
};
use wie_util::{ByteWrite, Result, WieError, read_generic, write_generic};

use crate::runtime::java::jvm_support::JavaClassDefinition;
use crate::runtime::{SVC_CATEGORY_JAVA, java::JavaSvcFunctions};

use super::{KtfJvmSupport, class_instance::JavaClassInstance, name::JavaFullName, oom, value::JavaValueExt};

pub struct JavaMethod {
    pub ptr_raw: u32,
    core: ArmCore,
}

impl JavaMethod {
    pub fn from_raw(ptr_raw: u32, core: &ArmCore) -> Self {
        Self { ptr_raw, core: core.clone() }
    }

    pub fn new<C, Context>(
        core: &mut ArmCore,
        jvm: &Jvm,
        ptr_class: u32,
        proto: JavaMethodProto<C>,
        context: Context,
        java_functions: JavaSvcFunctions,
    ) -> Result<Self>
    where
        C: ?Sized + 'static + Send,
        Context: Deref<Target = C> + DerefMut + Clone + 'static + Sync + Send,
    {
        let full_name = JavaFullName::new(0, proto.name.clone(), proto.descriptor.clone());
        let full_name_bytes = full_name.as_bytes();

        let ptr_name = Allocator::alloc(core, full_name_bytes.len() as u32)?;
        // See `JavaField::new` — same reason, same hazard.
        JavaFullName::forget(ptr_name);
        core.write_bytes(ptr_name, &full_name_bytes)?;

        let ptr_raw = Allocator::alloc(core, size_of::<RawJavaMethod>() as u32)?;

        let access_flags = proto.access_flags;
        let fn_method = Self::register_java_method(core, jvm, ptr_raw, proto, context, java_functions)?;

        let (fn_body, fn_body_native) = if access_flags.contains(MethodAccessFlags::NATIVE) {
            (0, fn_method)
        } else {
            (fn_method, 0)
        };

        write_generic(
            core,
            ptr_raw,
            RawJavaMethod {
                fn_body,
                ptr_class,
                fn_body_native_or_exception_table: fn_body_native,
                ptr_name,
                exception_table_count: 0,
                unk3: 0,
                index_in_vtable: 0, // to be filled later
                access_flags: access_flags.bits(),
                unk6: 0,
            },
        )?;

        tracing::trace!("Wrote method {} at {ptr_raw:#x}", full_name.name);

        Ok(Self::from_raw(ptr_raw, core))
    }

    pub fn write_vtable_index(&mut self, new_index: u16) -> Result<()> {
        let mut raw: RawJavaMethod = read_generic(&self.core, self.ptr_raw)?;

        raw.index_in_vtable = new_index;

        write_generic(&mut self.core, self.ptr_raw, raw)?;

        Ok(())
    }

    pub fn ptr_class(&self) -> u32 {
        let raw: RawJavaMethod = read_generic(&self.core, self.ptr_raw).unwrap();

        raw.ptr_class
    }

    pub fn name(&self) -> Result<JavaFullName> {
        Ok((*self.name_shared()?).clone())
    }

    /// The memoised name. Prefer this where the caller only reads it - `name()` has to
    /// clone two `String`s back out. See the `NAME_CACHE` note in `name.rs`.
    pub fn name_shared(&self) -> Result<Arc<JavaFullName>> {
        let raw: RawJavaMethod = read_generic(&self.core, self.ptr_raw)?;

        JavaFullName::from_ptr_shared(&self.core, raw.ptr_name)
    }

    pub async fn run(&self, args: Box<[JavaValue]>) -> Result<JavaValue> {
        let raw: RawJavaMethod = read_generic(&self.core, self.ptr_raw)?;
        // The memoised parse — `JavaType::parse(&self.descriptor())` here was a `String`
        // clone plus a fresh parse tree per call.
        let name = JavaFullName::from_ptr_shared(&self.core, raw.ptr_name)?;
        let return_type = name.method_type().1;

        let mut core = self.core.clone();

        let mut raw_args = Vec::with_capacity(args.len());
        for arg in args.iter() {
            if matches!(arg, JavaValue::Double(_) | JavaValue::Long(_)) {
                let (arg, arg_high) = arg.as_raw64();
                raw_args.push(arg);
                raw_args.push(arg_high);
            } else {
                raw_args.push(arg.as_raw());
            }
        }

        struct JavaMethodRunResult {
            result: u32,
            result_high: u32,
        }

        impl wie_core_arm::RunFunctionResult<JavaMethodRunResult> for JavaMethodRunResult {
            fn get(core: &ArmCore) -> Self {
                let result = core.read_param(0).unwrap();
                let result_high = core.read_param(1).unwrap();

                Self { result, result_high }
            }
        }

        let access_flags = MethodAccessFlags::from_bits_truncate(raw.access_flags);

        // Re-enters `run_function` if a Java catch handler matches the current ARM frame —
        // mirrors the trampoline path in `interface.rs::map_jump_result`, but for the
        // outermost frame whose caller is the Rust JVM rather than another ARM trampoline.
        async fn run_with_unwind(core: &mut ArmCore, mut pc: u32, mut args: Vec<u32>) -> Result<JavaMethodRunResult> {
            loop {
                match core.run_function::<JavaMethodRunResult>(pc, &args).await {
                    Ok(r) => return Ok(r),
                    Err(WieError::JavaExceptionUnwind {
                        context_base,
                        target,
                        next_pc,
                    }) => {
                        tracing::debug!("Resuming via exception restore: pc={next_pc:#x}, context_base={context_base:#x}, target={target:#x}");
                        pc = next_pc;
                        args = vec![context_base, target];
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        let result: JavaMethodRunResult = if access_flags.contains(MethodAccessFlags::NATIVE) {
            let arg_container = Allocator::alloc(&mut core, (raw_args.len() as u32) * 4)?;
            for (i, arg) in raw_args.iter().enumerate() {
                write_generic(&mut core, arg_container + (i * 4) as u32, *arg)?;
            }

            tracing::trace!("Calling native method: {:#x}", raw.fn_body_native_or_exception_table);
            let result = run_with_unwind(&mut core, raw.fn_body_native_or_exception_table, vec![0, arg_container]).await;

            Allocator::free(&mut core, arg_container, (raw_args.len() as u32) * 4)?;

            result?
        } else {
            let mut params = vec![0];
            params.extend(raw_args);

            tracing::trace!("Calling method: {:#x}", raw.fn_body);
            run_with_unwind(&mut core, raw.fn_body, params).await?
        };

        if matches!(return_type, JavaType::Double | JavaType::Long) {
            Ok(JavaValue::from_raw64(result.result, result.result_high, return_type))
        } else {
            Ok(JavaValue::from_raw(result.result, return_type, &core))
        }
    }

    fn exception_table(&self) -> Result<Vec<RawJavaMethodExceptionTableEntry>> {
        let raw: RawJavaMethod = read_generic(&self.core, self.ptr_raw)?;

        let mut result = Vec::with_capacity(raw.exception_table_count as _);

        if raw.exception_table_count == 0 {
            return Ok(result);
        }

        let mut cursor = raw.fn_body_native_or_exception_table;
        for _ in 0..raw.exception_table_count {
            let address = read_generic(&self.core, cursor)?;
            cursor += 4;

            result.push(read_generic(&self.core, address)?);
        }

        Ok(result)
    }

    pub async fn handle_exception(core: &mut ArmCore, jvm: &Jvm, exception: Box<dyn ClassInstance>) -> Result<JavaMethodResult> {
        tracing::warn!("Java exception thrown: {exception:?}");

        let current_java_exception_handler = KtfJvmSupport::current_java_exception_handler(core)?;

        if current_java_exception_handler == 0 {
            // `oom::to_wie_err`, not `JvmSupport::to_wie_err`: rendering a stack trace needs three more guest allocations, each `unwrap`ped, so
            // feeding it the out-of-memory exception would panic on the very path we are trying to survive.
            return Err(oom::to_wie_err(jvm, JavaError::JavaException(exception)).await);
        }

        let exception_handler: RawJavaExceptionHandler = read_generic(core, current_java_exception_handler)?;

        let method = JavaMethod::from_raw(exception_handler.ptr_method, core);
        let exception_table = method.exception_table()?;

        for entry in exception_table {
            if entry.from_pc <= exception_handler.current_pc && exception_handler.current_pc < entry.to_pc {
                let class = JavaClassDefinition::from_raw(entry.ptr_class, core);
                if entry.ptr_class == 0 || jvm.is_instance(&*exception, &class.name()?) {
                    let restore_context: u32 = read_generic(core, exception_handler.ptr_functions + 4)?;
                    let contexts_base = current_java_exception_handler + 24;

                    tracing::debug!(
                        "Java exception handler found: {:#x}, method: {:#x}",
                        entry.target,
                        exception_handler.ptr_method
                    );

                    return Err(WieError::JavaExceptionUnwind {
                        context_base: contexts_base,
                        target: entry.target,
                        next_pc: restore_context,
                    });
                }
            }
        }

        Err(oom::to_wie_err(jvm, JavaError::JavaException(exception)).await)
    }

    fn register_java_method<C, Context>(
        core: &mut ArmCore,
        jvm: &Jvm,
        ptr_method: u32,
        proto: JavaMethodProto<C>,
        context: Context,
        java_functions: JavaSvcFunctions,
    ) -> Result<u32>
    where
        C: ?Sized + 'static + Send,
        Context: Deref<Target = C> + DerefMut + Clone + 'static + Sync + Send,
    {
        let java_type = JavaType::parse(&proto.descriptor);
        let (parameter_types, return_type) = java_type.as_method();

        let mut parameter_types = parameter_types.to_vec();
        if !proto.access_flags.contains(MethodAccessFlags::STATIC) {
            // TODO proper flag handling
            parameter_types.insert(0, JavaType::Class("".into())); // TODO name
        }

        let proxy = JavaMethodProxy {
            jvm: jvm.clone(),
            proto,
            context,
            parameter_types,
            return_type: return_type.clone(),
        };

        // ★ The stub address IS the SVC id (`make_svc_stub` below), so this line is what
        //   turns an id in the SVC-mix gauge into a method name. ⚠ That gauge's histogram
        //   keys on `id & 2047` and 580 stubs are spread over 0xc000 bytes, so in THIS
        //   category two dozen methods share every bucket — to name a hot one, count by
        //   `ptr_method` instead (2026-09-18: a temporary census that way found
        //   `drawImage` at 40 % of every native Java call, counted twice).
        tracing::debug!("java stub {ptr_method:#x} -> {}{}", proxy.proto.name, proxy.proto.descriptor);

        let proxy = RegisteredFunctionHolder::new(proxy, &());
        java_functions
            .lock()
            .insert(ptr_method, Arc::new(Box::new(proxy) as Box<dyn RegisteredFunction>));

        core.make_svc_stub(SVC_CATEGORY_JAVA, ptr_method)
    }
}

#[async_trait::async_trait]
impl Method for JavaMethod {
    fn name(&self) -> String {
        let name = self.name().unwrap();

        name.name
    }

    fn descriptor(&self) -> String {
        let name = self.name().unwrap();

        name.descriptor
    }

    fn signature(&self) -> Arc<str> {
        self.name_shared().unwrap().signature().clone()
    }

    async fn run(&self, jvm: &Jvm, args: Box<[JavaValue]>) -> JvmResult<JavaValue> {
        let jvm_clone = jvm.clone();
        self.run(args)
            .or_else(async move |x| {
                Err(match x {
                    WieError::JavaException(ptr) => JavaError::JavaException(Box::new(JavaClassInstance::from_raw(ptr, &self.core))),
                    WieError::JavaExceptionUnwind { .. } => {
                        oom::java_error(&jvm_clone, &self.core, false, || "Java exception unwind crossed into JVM caller".into()).await
                    }
                    // Bound to `other` rather than matched with `_` so the message closure below captures a local instead of the scrutinee.
                    other => {
                        let allocation_failure = matches!(other, WieError::AllocationFailure);

                        oom::java_error(&jvm_clone, &self.core, allocation_failure, move || other.to_string()).await
                    }
                })
            })
            .await
    }

    fn access_flags(&self) -> MethodAccessFlags {
        let raw: RawJavaMethod = read_generic(&self.core, self.ptr_raw).unwrap();

        MethodAccessFlags::from_bits_truncate(raw.access_flags)
    }
}

impl Debug for JavaMethod {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaMethod").field("ptr_raw", &self.ptr_raw).finish()
    }
}

struct JavaMethodProxy<C, Context>
where
    C: ?Sized + Send,
    Context: Deref<Target = C> + DerefMut + Clone,
{
    jvm: Jvm,
    proto: JavaMethodProto<C>,
    context: Context,
    parameter_types: Vec<JavaType>,
    return_type: JavaType,
}

#[async_trait::async_trait]
impl<C, Context> EmulatedFunction<(), JavaMethodResult, ()> for JavaMethodProxy<C, Context>
where
    C: ?Sized + Send,
    Context: Deref<Target = C> + DerefMut + Clone + 'static + Sync + Send,
{
    async fn call(&self, core: &mut ArmCore, _: &mut ()) -> Result<JavaMethodResult> {
        let double_long_count = self
            .parameter_types
            .iter()
            .filter(|x| matches!(x, JavaType::Double | JavaType::Long))
            .count();

        let param_count = self.parameter_types.len() + double_long_count;

        // The raw words come straight from the guest (the argument block for a native
        // method, the registers/stack otherwise) into the typed values — no intermediate
        // `Vec<u32>`. This runs once per crossing, ~10,000 times a second on hero2.
        let native = self.proto.access_flags.contains(MethodAccessFlags::NATIVE);
        let param_base = if native { u32::get(core, 1) } else { 0 };
        let mut raw_at = |x: usize| -> wie_util::Result<u32> {
            if native {
                read_generic(core, param_base + (x as u32) * 4)
            } else {
                Ok(u32::get(core, x + 1))
            }
        };

        // ⛔ The inline buffer is an OPTIMISATION, never a limit (2026-09-20, review).
        // It replaced a per-crossing `Vec<u32>` and was guarded by a `debug_assert!` —
        // which is compiled out of every build that ships, so a method with more than 16
        // argument words would have sliced past the array and panicked on the emulator
        // thread with a message naming neither the method nor the real rule. A worker that
        // dies leaves the UI and the audio running (a frozen screen with music, the hardest
        // failure in this tree to diagnose), so the overflow spills to the heap instead:
        // `Vec::new()` does not allocate, so the common path is unchanged.
        let mut inline = [0u32; 16];
        let mut spill: Vec<u32> = Vec::new();
        let raw_args: &mut [u32] = if param_count <= inline.len() {
            &mut inline[..param_count]
        } else {
            spill.resize(param_count, 0);
            &mut spill
        };
        for (i, slot) in raw_args.iter_mut().enumerate() {
            *slot = raw_at(i)?;
        }

        let mut args = Vec::with_capacity(self.parameter_types.len());

        let mut it = raw_args.iter().copied();
        for param in self.parameter_types.iter() {
            let arg = it.next().unwrap();

            let value = if matches!(param, JavaType::Double | JavaType::Long) {
                let arg_high = it.next().unwrap();

                JavaValue::from_raw64(arg, arg_high, param)
            } else {
                JavaValue::from_raw(arg, param, core)
            };
            args.push(value);
        }

        let mut context = self.context.clone();
        let (_, lr) = core.read_pc_lr()?;

        let result = self.proto.body.call(&self.jvm, &mut context, args.into_boxed_slice()).await;
        if let Err(JavaError::JavaException(x)) = result {
            // if we executed this from rust code, we should propagate this down
            if lr == RUN_FUNCTION_LR {
                let java_exception = KtfJvmSupport::class_instance_raw(&x);
                return Err(WieError::JavaException(java_exception));
            }
            return JavaMethod::handle_exception(core, &self.jvm, x).await;
        }

        let result = if matches!(self.return_type, JavaType::Double | JavaType::Long) {
            let (result, result_high) = result.unwrap().as_raw64();
            JavaMethodResult::new(&[result, result_high], None)
        } else {
            JavaMethodResult::new(&[result.unwrap().as_raw()], None)
        };

        Ok(result)
    }
}

/// What a Java-interface SVC hands back to the guest: up to two words for `r0`/`r1`
/// (a 64-bit result, or the exception-unwind pair) and optionally a new pc. Inline —
/// this used to carry a `Vec<u32>`, one heap allocation per crossing for one or two
/// words.
pub struct JavaMethodResult {
    result: [u32; 2],
    len: u8,
    next_pc: Option<u32>,
}

impl JavaMethodResult {
    pub fn new(result: &[u32], next_pc: Option<u32>) -> Self {
        assert!(result.len() <= 2, "a Java SVC returns at most two words");
        let mut words = [0u32; 2];
        words[..result.len()].copy_from_slice(result);

        Self {
            result: words,
            len: result.len() as u8,
            next_pc,
        }
    }
}

impl ResultWriter<JavaMethodResult> for JavaMethodResult {
    fn write(self, core: &mut ArmCore, next_pc: u32) -> Result<()> {
        core.write_return_value(&self.result[..self.len as usize])?;

        if let Some(x) = self.next_pc {
            core.set_next_pc(x)?;
        } else {
            core.set_next_pc(next_pc)?;
        }

        Ok(())
    }
}
