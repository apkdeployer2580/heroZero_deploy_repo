#![allow(clippy::borrowed_box)] // We have get parameter by Box<T> to make ergonomic interface

use alloc::{borrow::ToOwned, boxed::Box, collections::BTreeMap, format, string::String, sync::Arc, vec::Vec};
use core::{
    fmt::Debug,
    hash::BuildHasher,
    iter,
    sync::atomic::{AtomicBool, Ordering},
};

use dyn_clone::clone_box;
use event_listener::{Event, EventListener};
use hashbrown::{DefaultHashBuilder, HashMap, HashSet};
use parking_lot::RwLock;

use java_constants::{ClassAccessFlags, FieldAccessFlags, MethodAccessFlags};

use crate::{
    Result,
    array_class_instance::{ArrayClassInstance, ArrayRawBuffer, ArrayRawBufferMut},
    class_definition::ClassDefinition,
    class_instance::ClassInstance,
    class_loader::{BootstrapClassLoader, BootstrapClassLoaderWrapper, Class, ClassLoaderWrapper, InitState, JavaClassLoaderWrapper},
    error::JavaError,
    field::Field,
    garbage_collector::{CollectingGuard, determine_garbage},
    invoke_arg::InvokeArg,
    method::Method,
    runtime::{JavaLangClass, JavaLangClassLoader, JavaLangString},
    thread::JvmThread,
    r#type::JavaType,
    value::JavaValue,
};

struct JvmInner {
    classes: RwLock<BTreeMap<String, Class>>,
    threads: RwLock<BTreeMap<u64, JvmThread>>,
    all_objects: RwLock<HashSet<Box<dyn ClassInstance>>>,
    /// Objects a piece of HOST code is holding onto, and which nothing the collector scans can
    /// see. See `Jvm::pin_object`.
    pinned_objects: RwLock<HashSet<Box<dyn ClassInstance>>>,
    string_pool: RwLock<BTreeMap<Vec<u16>, Box<dyn ClassInstance>>>,
    monitors: RwLock<BTreeMap<u64, Arc<Event>>>,
    monitor_hasher: DefaultHashBuilder,
    get_current_thread_id: Box<dyn Fn() -> u64 + Sync + Send>,
    bootstrap_class_loader: Box<dyn BootstrapClassLoader>,
    bootstrapping: AtomicBool,
    /// The backend's conservative root scanner, if it installed one. See
    /// [`Jvm::set_conservative_root_scanner`].
    conservative_root_scanner: RwLock<Option<ConservativeRootScanner>>,
    memo: Memo,
}

/// ★ heroZero fork (2026-10-27): what a field or method reference RESOLVED to, so the next
/// execution of the same `getfield` / `invokevirtual` / native field read does not resolve it
/// again. Resolution walked the class chain comparing names, boxing a clone of every class and
/// field it passed and building the superclass name as a fresh `String` per step - on every
/// field access and call. Profiled in 영웅서기 Zero's field (probe, map 44): the lookups and their
/// allocations were most of the game's frame.
///
/// The key is the ADDRESSES of the strings that named the reference (a constant-pool entry's
/// `Arc<String>`, a native caller's literal) plus, for a call or read on an object, the identity
/// of its class (`ClassInstance::class_identity`). An address is only a hint: every hit compares
/// the stored names with the given ones, so a string freed and its address reused for another
/// name misses and is resolved again. A class identity is pinned by the entry (it holds the
/// class), so it cannot be reused while the entry exists. Only successful resolutions are kept.
/// `MEMO_MAX` bounds a map a caller feeding fresh strings would grow; past it the map starts over.
#[derive(Default)]
struct Memo {
    /// `getfield` / `putfield`: (class named, name, descriptor) → the field, from the named class.
    fields: RwLock<HashMap<MemoKey, MemoEntry<Arc<dyn Field>>>>,
    /// `getstatic` / `putstatic`: → (declaring class, field).
    statics: RwLock<HashMap<MemoKey, MemoEntry<(Class, Arc<dyn Field>)>>>,
    /// `invokestatic`: → (declaring class, method).
    static_methods: RwLock<HashMap<MemoKey, MemoEntry<(Class, Arc<dyn Method>)>>>,
    /// `invokevirtual` / `invokeinterface`: (runtime class, name, descriptor) → (that class, method).
    virtual_methods: RwLock<HashMap<MemoKey, MemoEntry<(Class, Arc<dyn Method>)>>>,
    /// `Jvm::get_field` / `put_field` by name: (runtime class, name, descriptor) → (the class, field).
    instance_fields: RwLock<HashMap<MemoKey, MemoEntry<(Box<dyn ClassDefinition>, Arc<dyn Field>)>>>,
}

type MemoKey = (usize, usize, usize);

const MEMO_MAX: usize = 16 * 1024;

struct MemoEntry<T> {
    /// The named class, for an entry keyed by a class NAME (`None`: keyed by a class identity).
    class: Option<Box<str>>,
    name: Box<str>,
    descriptor: Box<str>,
    value: T,
}

impl<T: Clone> MemoEntry<T> {
    fn get(map: &RwLock<HashMap<MemoKey, Self>>, key: MemoKey, class: Option<&str>, name: &str, descriptor: &str) -> Option<T> {
        let map = map.read();
        let entry = map.get(&key)?;
        (entry.class.as_deref() == class && &*entry.name == name && &*entry.descriptor == descriptor).then(|| entry.value.clone())
    }

    fn put(map: &RwLock<HashMap<MemoKey, Self>>, key: MemoKey, class: Option<&str>, name: &str, descriptor: &str, value: T) {
        let mut map = map.write();
        if map.len() >= MEMO_MAX {
            map.clear();
        }
        map.insert(
            key,
            Self {
                class: class.map(Into::into),
                name: name.into(),
                descriptor: descriptor.into(),
                value,
            },
        );
    }
}

fn memo_key(first: usize, name: &str, descriptor: &str) -> MemoKey {
    (first, name.as_ptr() as usize, descriptor.as_ptr() as usize)
}

/// Hands the collector every raw address the backend's own machine state is holding.
///
/// ★★ WHY A JVM NEEDS THIS AT ALL (2026-09-19). A bytecode interpreter keeps java locals
/// in frames the collector walks (`JvmThread::iter_frame`). A backend that runs AOT-compiled
/// guest code does not: hero2's locals live in ARM registers and on an emulated stack, and
/// nothing in this crate can see them. Four collections' worth of device crashes came from
/// that one gap — the last of them (지옥불, S9, 2026-09-19) freed an object whose only
/// reference was `r4` of a frame parked by a nested guest call.
///
/// The scanner is CONSERVATIVE: it offers words that MIGHT be object addresses, and
/// [`determine_garbage`] keeps any object whose [`ClassInstance::gc_raw_address`] matches.
/// A false match keeps a dead object one collection longer; a missed reference costs the
/// player their session. Only one of those is recoverable.
pub type ConservativeRootScanner = Box<dyn Fn(&mut dyn FnMut(u64)) + Sync + Send>;

#[derive(Clone)]
pub struct Jvm {
    inner: Arc<JvmInner>,
}

impl Jvm {
    pub async fn new<C, F>(bootstrap_class_loader: C, get_current_thread_id: F, properties: BTreeMap<&str, &str>) -> Result<Self>
    where
        C: BootstrapClassLoader + 'static,
        F: Fn() -> u64 + 'static + Sync + Send,
    {
        let jvm = Self {
            inner: Arc::new(JvmInner {
                classes: RwLock::new(BTreeMap::new()),
                threads: RwLock::new(BTreeMap::new()),
                all_objects: RwLock::new(HashSet::new()),
                pinned_objects: RwLock::new(HashSet::new()),
                string_pool: RwLock::new(BTreeMap::new()),
                monitors: RwLock::new(BTreeMap::new()),
                monitor_hasher: DefaultHashBuilder::default(),
                get_current_thread_id: Box::new(get_current_thread_id),
                bootstrap_class_loader: Box::new(bootstrap_class_loader),
                bootstrapping: AtomicBool::new(true),
                conservative_root_scanner: RwLock::new(None),
                memo: Memo::default(),
            }),
        };

        // load bootstrap classes
        let bootstrap_classes = ["java/lang/Object", "java/lang/Thread", "[B", "java/lang/Class"];
        for class_name in bootstrap_classes.iter() {
            let class_definition = jvm.inner.bootstrap_class_loader.load_class(&jvm, class_name).await?.unwrap();
            let class = Class::new(class_definition, None);

            jvm.register_class_internal(class, None).await?;
        }

        // init startup thread
        jvm.attach_thread()?;

        // set java class for bootstrap classes
        let classes = jvm.inner.classes.read().values().cloned().collect::<Vec<_>>();
        for class in classes {
            let java_class = JavaLangClass::from_rust_class(&jvm, &*class.definition, None).await?;
            class.set_java_class(java_class);
        }

        // init properties
        for (key, value) in properties {
            let key = JavaLangString::from_rust_string(&jvm, key).await?;
            let value = JavaLangString::from_rust_string(&jvm, value).await?;

            let _: Option<Box<dyn ClassInstance>> = jvm
                .invoke_static(
                    "java/lang/System",
                    "setProperty",
                    "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Object;",
                    (key, value),
                )
                .await?;
        }

        // load system class loader
        JavaLangClassLoader::get_system_class_loader(&jvm).await?;

        jvm.inner.bootstrapping.store(false, Ordering::Relaxed);

        Ok(jvm)
    }

    #[async_recursion::async_recursion]
    pub async fn instantiate_class(&self, class_name: &str) -> Result<Box<dyn ClassInstance>> {
        tracing::trace!("Instantiate {class_name}");

        let class = self.resolve_class(class_name).await?;

        let access_flags = class.definition.access_flags();
        if access_flags.contains(ClassAccessFlags::INTERFACE) || access_flags.contains(ClassAccessFlags::ABSTRACT) {
            return Err(self
                .exception(
                    "java/lang/InstantiationError",
                    &format!("Cannot instantiate abstract class or interface: {}", class_name),
                )
                .await);
        }

        self.ensure_initialized(&class).await?;

        let instance = class.definition.instantiate(self).await?;

        let thread_id = (self.inner.get_current_thread_id)();
        let mut threads = self.inner.threads.write();
        let thread = threads.get_mut(&thread_id).unwrap();

        thread.top_frame_mut().local_variables_mut().push(instance.clone());
        self.inner.all_objects.write().insert(instance.clone());

        Ok(instance)
    }

    pub async fn new_class<T>(&self, class_name: &str, init_descriptor: &str, init_args: T) -> Result<Box<dyn ClassInstance>>
    where
        T: InvokeArg,
    {
        let instance = self.instantiate_class(class_name).await?;

        let _: () = self.invoke_special(&instance, class_name, "<init>", init_descriptor, init_args).await?;

        Ok(instance)
    }

    pub async fn instantiate_array(&self, element_type_name: &str, length: usize) -> Result<Box<dyn ClassInstance>> {
        tracing::trace!("Instantiate array of {element_type_name} with length {length}");

        let class_name = format!("[{element_type_name}");

        let class = self.resolve_class(&class_name).await?.definition;
        let array_class = class.as_array_class_definition().unwrap();

        let instance = array_class.instantiate_array(self, length).await?;

        let thread_id = (self.inner.get_current_thread_id)();
        let mut threads = self.inner.threads.write();
        let thread = threads.get_mut(&thread_id).unwrap();

        thread.top_frame_mut().local_variables_mut().push(instance.clone());
        self.inner.all_objects.write().insert(instance.clone());

        Ok(instance)
    }

    pub async fn get_static_field<T>(&self, class_name: &str, name: &str, descriptor: &str) -> Result<T>
    where
        T: From<JavaValue>,
    {
        tracing::trace!("Get static field {class_name}.{name}:{descriptor}");

        if let Some((declaring_class, field)) = self.memo_static(class_name, name, descriptor) {
            self.ensure_initialized(&declaring_class).await?;
            return Ok(declaring_class.definition.get_static_field(&*field)?.into());
        }

        let class = self.resolve_class(class_name).await?;

        if let Some((declaring_class, field)) = self.resolve_field(&class, name, descriptor) {
            if !field.access_flags().contains(FieldAccessFlags::STATIC) {
                return Err(self
                    .exception("java/lang/IncompatibleClassChangeError", &format!("{class_name}.{name}:{descriptor}"))
                    .await);
            }

            self.ensure_initialized(&declaring_class).await?;

            let field: Arc<dyn Field> = field.into();
            self.remember_static(class_name, name, descriptor, (declaring_class.clone(), field.clone()));
            Ok(declaring_class.definition.get_static_field(&*field)?.into())
        } else {
            Err(self
                .exception("java/lang/NoSuchFieldError", &format!("{class_name}.{name}:{descriptor}"))
                .await)
        }
    }

    pub async fn put_static_field<T>(&self, class_name: &str, name: &str, descriptor: &str, value: T) -> Result<()>
    where
        T: Into<JavaValue> + Debug,
    {
        tracing::trace!("Put static field {class_name}.{name}:{descriptor} = {value:?}");

        if let Some((declaring_class, field)) = self.memo_static(class_name, name, descriptor) {
            self.ensure_initialized(&declaring_class).await?;
            return declaring_class.definition.put_static_field(&*field, value.into());
        }

        let class = self.resolve_class(class_name).await?;

        if let Some((declaring_class, field)) = self.resolve_field(&class, name, descriptor) {
            if !field.access_flags().contains(FieldAccessFlags::STATIC) {
                return Err(self
                    .exception("java/lang/IncompatibleClassChangeError", &format!("{class_name}.{name}:{descriptor}"))
                    .await);
            }

            self.ensure_initialized(&declaring_class).await?;

            let field: Arc<dyn Field> = field.into();
            self.remember_static(class_name, name, descriptor, (declaring_class.clone(), field.clone()));
            declaring_class.definition.put_static_field(&*field, value.into())
        } else {
            Err(self
                .exception("java/lang/NoSuchFieldError", &format!("{class_name}.{name}:{descriptor}"))
                .await)
        }
    }

    pub async fn get_field<T>(&self, instance: &Box<dyn ClassInstance>, name: &str, descriptor: &str) -> Result<T>
    where
        T: From<JavaValue>,
    {
        tracing::trace!("Get field {}.{name}:{descriptor}", instance.class_definition().name());

        if let Some(field) = self.instance_field(&**instance, name, descriptor)? {
            return Ok(instance.get_field(&*field)?.into());
        }

        if let Some(value) = instance.get_field_by_name(self, name, descriptor)? {
            Ok(value.into())
        } else {
            Err(self
                .exception(
                    "java/lang/NoSuchFieldError",
                    &format!("{}.{}:{}", instance.class_definition().name(), name, descriptor),
                )
                .await)
        }
    }

    /// ★ heroZero fork (2026-10-24): `getfield class.name:descriptor` - the field resolved from
    /// `class`, the class the instruction names (JVMS §5.4.3.2), not from the object's runtime
    /// class as [`get_field`](Self::get_field) does. When a subclass declares a field with its
    /// superclass's name and descriptor they are two fields, and the superclass's code reaches its
    /// own (영웅서기 Zero: the window `o`'s first-paint flag `e:Z` vs the inventory `ds`'s
    /// "on the bag tabs" `e:Z`). Same cost as `get_field`: one class lookup, then the same walk.
    pub async fn get_field_from<T>(&self, instance: &Box<dyn ClassInstance>, class_name: &str, name: &str, descriptor: &str) -> Result<T>
    where
        T: From<JavaValue>,
    {
        tracing::trace!("Get field {class_name}.{name}:{descriptor} of {}", instance.class_definition().name());

        if let Some(field) = self.find_field_from(class_name, name, descriptor).await? {
            Ok(instance.get_field(&*field)?.into())
        } else {
            Err(self
                .exception("java/lang/NoSuchFieldError", &format!("{class_name}.{name}:{descriptor}"))
                .await)
        }
    }

    /// ★ heroZero fork (2026-10-24): `putfield` - see [`get_field_from`](Self::get_field_from).
    pub async fn put_field_from<T>(&self, instance: &mut Box<dyn ClassInstance>, class_name: &str, name: &str, descriptor: &str, value: T) -> Result<()>
    where
        T: Into<JavaValue> + Debug,
    {
        tracing::trace!("Put field {class_name}.{name}:{descriptor} of {} = {value:?}", instance.class_definition().name());

        if let Some(field) = self.find_field_from(class_name, name, descriptor).await? {
            instance.put_field(&*field, value.into())
        } else {
            Err(self
                .exception("java/lang/NoSuchFieldError", &format!("{class_name}.{name}:{descriptor}"))
                .await)
        }
    }

    async fn find_field_from(&self, class_name: &str, name: &str, descriptor: &str) -> Result<Option<Arc<dyn Field>>> {
        let key = memo_key(class_name.as_ptr() as usize, name, descriptor);
        if let Some(field) = MemoEntry::get(&self.inner.memo.fields, key, Some(class_name), name, descriptor) {
            return Ok(Some(field));
        }
        let class = match self.get_class(class_name) {
            Some(x) => x,
            None => self.resolve_class(class_name).await?,
        };
        let Some(field) = self.find_field(&*class.definition, name, descriptor)? else {
            return Ok(None);
        };
        let field: Arc<dyn Field> = field.into();
        MemoEntry::put(&self.inner.memo.fields, key, Some(class_name), name, descriptor, field.clone());
        Ok(Some(field))
    }

    /// ★ heroZero fork (2026-10-27): the field `name:descriptor` of `instance`'s class chain, through
    /// the memo - `None` when the backend gives no class identity (it resolves its own way, through
    /// `get_field_by_name`) or there is no such field.
    fn instance_field(&self, instance: &dyn ClassInstance, name: &str, descriptor: &str) -> Result<Option<Arc<dyn Field>>> {
        let Some(id) = instance.class_identity() else {
            return Ok(None);
        };
        let key = memo_key(id, name, descriptor);
        if let Some((_, field)) = MemoEntry::get(&self.inner.memo.instance_fields, key, None, name, descriptor) {
            return Ok(Some(field));
        }
        let class = instance.class_definition();
        let Some(field) = self.find_field(&*class, name, descriptor)? else {
            return Ok(None);
        };
        let field: Arc<dyn Field> = field.into();
        MemoEntry::put(&self.inner.memo.instance_fields, key, None, name, descriptor, (class, field.clone()));
        Ok(Some(field))
    }

    fn memo_static(&self, class_name: &str, name: &str, descriptor: &str) -> Option<(Class, Arc<dyn Field>)> {
        MemoEntry::get(&self.inner.memo.statics, memo_key(class_name.as_ptr() as usize, name, descriptor), Some(class_name), name, descriptor)
    }

    fn remember_static(&self, class_name: &str, name: &str, descriptor: &str, value: (Class, Arc<dyn Field>)) {
        MemoEntry::put(&self.inner.memo.statics, memo_key(class_name.as_ptr() as usize, name, descriptor), Some(class_name), name, descriptor, value);
    }

    pub async fn put_field<T>(&self, instance: &mut Box<dyn ClassInstance>, name: &str, descriptor: &str, value: T) -> Result<()>
    where
        T: Into<JavaValue> + Debug,
    {
        tracing::trace!("Put field {}.{name}:{descriptor} = {value:?}", instance.class_definition().name());

        if let Some(field) = self.instance_field(&**instance, name, descriptor)? {
            return instance.put_field(&*field, value.into());
        }

        if instance.put_field_by_name(self, name, descriptor, value.into())? {
            Ok(())
        } else {
            Err(self
                .exception(
                    "java/lang/NoSuchFieldError",
                    &format!("{}.{}:{}", instance.class_definition().name(), name, descriptor),
                )
                .await)
        }
    }

    pub async fn invoke_static<T, U>(&self, class_name: &str, name: &str, descriptor: &str, args: T) -> Result<U>
    where
        T: InvokeArg,
        U: From<JavaValue>,
    {
        let args = args.into_arg();

        tracing::trace!("Invoke static {class_name}.{name}:{descriptor}({args:?})");

        let key = memo_key(class_name.as_ptr() as usize, name, descriptor);
        if let Some((declaring_class, method)) = MemoEntry::get(&self.inner.memo.static_methods, key, Some(class_name), name, descriptor) {
            self.ensure_initialized(&declaring_class).await?;
            return Ok(self.execute_method(&declaring_class, None, &*method, args).await?.into());
        }

        let class = self.resolve_class(class_name).await?;

        if let Some((declaring_class, method)) = self.resolve_method(&class, name, descriptor) {
            if !method.access_flags().contains(MethodAccessFlags::STATIC) {
                return Err(self
                    .exception("java/lang/IncompatibleClassChangeError", &format!("{class_name}.{name}:{descriptor}"))
                    .await);
            }

            self.ensure_initialized(&declaring_class).await?;

            let method: Arc<dyn Method> = method.into();
            MemoEntry::put(&self.inner.memo.static_methods, key, Some(class_name), name, descriptor, (declaring_class.clone(), method.clone()));
            Ok(self.execute_method(&declaring_class, None, &*method, args).await?.into())
        } else {
            tracing::error!("No such method: {class_name}.{name}:{descriptor}");

            Err(self
                .exception("java/lang/NoSuchMethodError", &format!("{class_name}.{name}:{descriptor}"))
                .await)
        }
    }

    pub async fn invoke_virtual<T, U>(&self, instance: &Box<dyn ClassInstance>, name: &str, descriptor: &str, args: T) -> Result<U>
    where
        T: InvokeArg,
        U: From<JavaValue>,
    {
        let args = args.into_arg();
        tracing::trace!("Invoke virtual {}.{name}:{descriptor}({args:?})", instance.class_definition().name());

        let key = instance.class_identity().map(|id| memo_key(id, name, descriptor));
        if let Some(key) = key
            && let Some((class, method)) = MemoEntry::get(&self.inner.memo.virtual_methods, key, None, name, descriptor)
        {
            let args = iter::once(JavaValue::Object(Some(clone_box(&**instance))))
                .chain(args.into_vec())
                .collect::<Vec<_>>();
            return Ok(self
                .execute_method(&class, Some(instance.clone()), &*method, args.into_boxed_slice())
                .await?
                .into());
        }

        let class = instance.class_definition();
        let method = self.find_virtual_method(&*class, name, descriptor, false)?;
        if let Some(x) = method {
            let args = iter::once(JavaValue::Object(Some(clone_box(&**instance))))
                .chain(args.into_vec())
                .collect::<Vec<_>>();

            let class = self.resolve_class(&class.name()).await?; // TODO we're resolving class twice
            let x: Arc<dyn Method> = x.into();
            if let Some(key) = key {
                MemoEntry::put(&self.inner.memo.virtual_methods, key, None, name, descriptor, (class.clone(), x.clone()));
            }
            Ok(self
                .execute_method(&class, Some(instance.clone()), &*x, args.into_boxed_slice())
                .await?
                .into())
        } else {
            tracing::error!("No such method: {}.{name}:{descriptor}", class.name());

            Err(self
                .exception("java/lang/NoSuchMethodError", &format!("{}.{}:{}", class.name(), name, descriptor))
                .await)
        }
    }

    // non-virtual
    #[async_recursion::async_recursion]
    pub async fn invoke_special<T, U>(&self, instance: &Box<dyn ClassInstance>, class_name: &str, name: &str, descriptor: &str, args: T) -> Result<U>
    where
        T: InvokeArg,
        U: From<JavaValue>,
    {
        let args = args.into_arg();
        tracing::trace!("Invoke special {class_name}.{name}:{descriptor}({args:?})");

        let class = self.resolve_class(class_name).await?;
        // ★ heroZero fork (2026-10-21): resolve UP the superclass chain (JVMS §5.4.3.3), not in
        // the named class alone. javac names the IMMEDIATE superclass in a `super.m()` call,
        // whichever ancestor actually declares `m` — 영웅서기 Zero's canvas `f` calls
        // `super.getWidth()` as `invokespecial Canvas.getWidth:()I`, and `getWidth` lives on
        // `Displayable`. Upstream looked only in `Canvas` and threw NoSuchMethodError at
        // `startApp` (first thing the game does). The method runs in its DECLARING class.
        let resolved = self.resolve_special_method(&class, name, descriptor);

        if let Some((declaring_class, method)) = resolved {
            let args = iter::once(JavaValue::Object(Some(clone_box(&**instance))))
                .chain(args.into_vec())
                .collect::<Vec<_>>();

            if method.access_flags().contains(MethodAccessFlags::STATIC) {
                return Err(self
                    .exception("java/lang/IncompatibleClassChangeError", &format!("{class_name}.{name}:{descriptor}"))
                    .await);
            }

            Ok(self
                .execute_method(&declaring_class, Some(instance.clone()), &*method, args.into_boxed_slice())
                .await?
                .into())
        } else {
            Err(self
                .exception("java/lang/NoSuchMethodError", &format!("{class_name}.{name}:{descriptor}"))
                .await)
        }
    }

    #[async_recursion::async_recursion]
    pub async fn store_array<T, U>(&self, array: &mut Box<dyn ClassInstance>, offset: usize, values: T) -> Result<()>
    where
        T: IntoIterator<Item = U> + Send,
        U: Into<JavaValue>,
    {
        tracing::trace!("Store array {} at offset {offset}", array.class_definition().name());

        let values = values.into_iter().map(|x| x.into()).collect::<Vec<_>>();

        let array_size = self.array_length(array).await?;
        if offset + values.len() > array_size {
            return Err(self
                .exception(
                    "java/lang/ArrayIndexOutOfBoundsException",
                    &format!("{} > {}", offset + values.len(), array_size),
                )
                .await);
        }

        let array = array.as_array_instance_mut();

        if let Some(array) = array {
            array.store(offset, values.into_boxed_slice())?;

            Ok(())
        } else {
            Err(self.exception("java/lang/IllegalArgumentException", "Not an array").await)
        }
    }

    pub async fn load_array<T>(&self, array: &Box<dyn ClassInstance>, offset: usize, count: usize) -> Result<Vec<T>>
    where
        T: From<JavaValue>,
    {
        tracing::trace!("Load array {} at offset {offset}", array.class_definition().name());

        let array_size = self.array_length(array).await?;
        if offset + count > array_size {
            return Err(self
                .exception(
                    "java/lang/ArrayIndexOutOfBoundsException",
                    &format!("{} > {}", offset + count, array_size),
                )
                .await);
        }

        let array = array.as_array_instance();

        if let Some(array) = array {
            let values = array.load(offset, count)?;

            Ok(iter::IntoIterator::into_iter(values).map(|x| x.into()).collect::<Vec<_>>())
        } else {
            Err(self.exception("java/lang/IllegalArgumentException", "Not an array").await)
        }
    }

    pub async fn array_raw_buffer(&self, array: &Box<dyn ClassInstance>) -> Result<Box<dyn ArrayRawBuffer>> {
        let array = array.as_array_instance();

        if let Some(array) = array {
            array.raw_buffer()
        } else {
            Err(self.exception("java/lang/IllegalArgumentException", "Not an array").await)
        }
    }

    pub async fn array_raw_buffer_mut(&self, array: &mut Box<dyn ClassInstance>) -> Result<Box<dyn ArrayRawBufferMut>> {
        let array = array.as_array_instance_mut();

        if let Some(array) = array {
            array.raw_buffer_mut()
        } else {
            Err(self.exception("java/lang/IllegalArgumentException", "Not an array").await)
        }
    }

    pub async fn array_length(&self, array: &Box<dyn ClassInstance>) -> Result<usize> {
        tracing::trace!("Get array length {}", array.class_definition().name());

        let array = array.as_array_instance();

        if let Some(array) = array {
            Ok(array.length())
        } else {
            Err(self.exception("java/lang/IllegalArgumentException", "Not an array").await)
        }
    }

    pub async fn array_element_type(&self, array: &Box<dyn ClassInstance>) -> Result<JavaType> {
        tracing::trace!("Get array element type {}", array.class_definition().name());

        let array = array.as_array_instance();

        if let Some(array) = array {
            let class = ArrayClassInstance::class_definition(array);

            let type_name = &class.name()[1..]; // TODO can we store JavaType on class?

            Ok(JavaType::parse(type_name))
        } else {
            Err(self.exception("java/lang/IllegalArgumentException", "Not an array").await)
        }
    }

    pub fn destroy(&self, instance: Box<dyn ClassInstance>) -> Result<()> {
        tracing::debug!("Destroy {}", instance.class_definition().name());

        self.inner.all_objects.write().remove(&instance);
        instance.destroy();

        Ok(())
    }

    // JVMS 5.1 string interning: equal string literals (and String.intern results) share one instance
    pub async fn intern_string(&self, value: &str) -> Result<Box<dyn ClassInstance>> {
        let key = value.encode_utf16().collect::<Vec<_>>();
        if let Some(interned) = self.inner.string_pool.read().get(&key) {
            return Ok(clone_box(&**interned));
        }

        let instance = JavaLangString::from_rust_string(self, value).await?;

        Ok(self.intern_or_get(key, instance))
    }

    // String.intern(): the receiver itself is pooled and returned when it is the first with its value (JVMS String spec)
    pub fn intern_string_instance(&self, instance: Box<dyn ClassInstance>, value: &[u16]) -> Box<dyn ClassInstance> {
        if let Some(interned) = self.inner.string_pool.read().get(value) {
            return clone_box(&**interned);
        }

        self.intern_or_get(value.to_owned(), instance)
    }

    // insert under the write lock, re-checking so concurrent interners converge on one canonical instance
    fn intern_or_get(&self, key: Vec<u16>, instance: Box<dyn ClassInstance>) -> Box<dyn ClassInstance> {
        let mut pool = self.inner.string_pool.write();
        let interned = pool.entry(key).or_insert(instance);

        clone_box(&**interned)
    }

    pub(crate) fn interned_strings(&self) -> Vec<Box<dyn ClassInstance>> {
        self.inner.string_pool.read().values().map(|x| clone_box(&**x)).collect()
    }

    /// Keep this object alive until [`Jvm::unpin_object`], no matter what the collector can see.
    ///
    /// ★ The root set is "class statics, stack frames, interned strings". None of those covers
    /// an object that only HOST code is holding - a `ClassInstanceRef` living in a Rust struct
    /// across an await. `Thread::start` is the case that forced this: it hands the new `Thread`
    /// to a spawn callback and returns, so between that return and the callback's first
    /// instruction the object is unreachable by every root the collector knows about. Measured on
    /// device 2026-09-10: the KTF backend then freed it, and `Thread::run` read `target` out of a
    /// corpse. On a backend where instances are emulated guest memory the allocator reuses, that
    /// is not a leak - it is two live objects sharing an address.
    ///
    /// Pins are a set, so pinning twice and unpinning once still releases; nothing needs a count
    /// today and a count would hide a mismatched call.
    pub fn pin_object(&self, instance: Box<dyn ClassInstance>) {
        self.inner.pinned_objects.write().insert(instance);
    }

    pub fn unpin_object(&self, instance: &dyn ClassInstance) {
        self.inner.pinned_objects.write().remove(&clone_box(instance));
    }

    pub(crate) fn pinned_objects(&self) -> Vec<Box<dyn ClassInstance>> {
        self.inner.pinned_objects.read().iter().map(|x| clone_box(&**x)).collect()
    }

    pub fn has_class(&self, class_name: &str) -> bool {
        self.inner.classes.read().contains_key(class_name)
    }

    pub fn get_class(&self, class_name: &str) -> Option<Class> {
        self.inner.classes.read().get(class_name).cloned()
    }

    pub fn object_listen(&self, obj: &Box<dyn ClassInstance>) -> EventListener {
        self.get_or_create_monitor(obj).listen()
    }

    pub async fn object_wait(&self, obj: &Box<dyn ClassInstance>) -> Result<()> {
        self.object_listen(obj).await;

        Ok(())
    }

    pub fn object_notify(&self, obj: &Box<dyn ClassInstance>, count: usize) {
        let monitor = self.get_or_create_monitor(obj);
        monitor.notify(count);
    }

    #[async_recursion::async_recursion]
    pub async fn resolve_class(&self, class_name: &str) -> Result<Class> {
        self.resolve_class_internal(class_name, None).await
    }

    #[async_recursion::async_recursion]
    async fn resolve_class_internal(&self, class_name: &str, class_loader_wrapper: Option<&dyn ClassLoaderWrapper>) -> Result<Class> {
        tracing::trace!("Resolving class {class_name}");
        let class = self.inner.classes.read().get(class_name).cloned();

        if let Some(x) = class {
            return Ok(x);
        }

        if class_name.starts_with('[') {
            let stripped_name = class_name.trim_start_matches('[');
            if stripped_name.starts_with('L') {
                self.resolve_class(&stripped_name[1..stripped_name.len() - 1]).await?;
                // ensure element type is loaded
            }
        }

        let class_loader_wrapper: &dyn ClassLoaderWrapper = if let Some(x) = class_loader_wrapper {
            x
        } else if self.inner.bootstrapping.load(Ordering::Relaxed) {
            &BootstrapClassLoaderWrapper::new(&*self.inner.bootstrap_class_loader)
        } else {
            &JavaClassLoaderWrapper::new(self.current_class_loader().await?)
        };

        let class = self.load_class(class_name, class_loader_wrapper).await?;

        // loader wrappers may build a fresh Class around an already-registered definition,
        // so return the registry copy to keep init state shared
        if let Some(registered) = self.get_class(class_name) {
            return Ok(registered);
        }

        Ok(class)
    }

    async fn load_class(&self, class_name: &str, class_loader_wrapper: &dyn ClassLoaderWrapper) -> Result<Class> {
        tracing::debug!("Loading class {class_name}");

        let class = class_loader_wrapper.load_class(self, class_name).await?;

        if class.is_none() {
            tracing::error!("No such class: {class_name}");

            return Err(self.exception("java/lang/NoClassDefFoundError", class_name).await);
        }

        tracing::debug!("Loaded class {class_name}");

        Ok(class.unwrap())
    }

    #[allow(clippy::type_complexity)]
    fn find_calling_class(&self) -> Result<Option<(Class, Option<Box<dyn ClassInstance>>)>> {
        let thread_id = (self.inner.get_current_thread_id)();

        let threads = self.inner.threads.read();
        let thread = threads.get(&thread_id).unwrap();

        Ok(thread.top_java_frame().map(|x| (x.class.clone(), x.class_instance.clone())))
    }

    pub async fn register_class(
        &self,
        class: Box<dyn ClassDefinition>,
        class_loader: Option<Box<dyn ClassInstance>>,
    ) -> Result<Option<Box<dyn ClassInstance>>> {
        tracing::debug!("Register class {}", class.name());

        let java_class = Some(JavaLangClass::from_rust_class(self, &*class, class_loader.clone()).await?);

        let class = Class::new(class, java_class.clone());

        if let Some(x) = class_loader {
            self.register_class_internal(class, Some(&JavaClassLoaderWrapper::new(x))).await?;
        } else {
            self.register_class_internal(class, None).await?;
        };

        Ok(java_class)
    }

    pub fn is_instance(&self, instance: &dyn ClassInstance, class_name: &str) -> bool {
        self.is_type_assignable(
            &JavaType::from_class_name(&instance.class_definition().name()),
            &JavaType::from_class_name(class_name),
        )
    }

    // aastore: whether value may be stored into array (JVMS 6.5 aastore)
    pub fn array_store_allowed(&self, array: &dyn ClassInstance, value: &dyn ClassInstance) -> bool {
        let JavaType::Array(component) = JavaType::parse(&array.class_definition().name()) else {
            return true;
        };

        self.is_type_assignable(&JavaType::from_class_name(&value.class_definition().name()), &component)
    }

    // JVMS 4.10.3 subtyping, including array covariance
    fn is_type_assignable(&self, source: &JavaType, target: &JavaType) -> bool {
        if source == target {
            return true;
        }

        match (source, target) {
            (JavaType::Array(source_component), JavaType::Array(target_component)) => self.is_type_assignable(source_component, target_component),
            // every array type is a subtype of Object, Cloneable, and java.io.Serializable (JLS 4.10.3)
            (JavaType::Array(_), JavaType::Class(name)) => {
                name == "java/lang/Object" || name == "java/lang/Cloneable" || name == "java/io/Serializable"
            }
            (JavaType::Class(source), JavaType::Class(target)) => self.is_class_assignable(source, target),
            _ => false,
        }
    }

    fn is_class_assignable(&self, source: &str, target: &str) -> bool {
        match self.get_class(source) {
            Some(class) => self.is_inherited_from(&*class.definition, target),
            None => false,
        }
    }

    pub fn is_inherited_from(&self, class: &dyn ClassDefinition, class_name: &str) -> bool {
        if class.name() == class_name {
            return true;
        }

        for interface in class.interface_names() {
            if interface == class_name {
                return true;
            }

            let interface_class = self.inner.classes.read().get(&interface).unwrap().definition.clone();
            if self.is_inherited_from(&*interface_class, class_name) {
                return true;
            }
        }

        if let Some(super_class) = class.super_class_name() {
            let super_class = self.inner.classes.read().get(&super_class).unwrap().definition.clone();
            self.is_inherited_from(&*super_class, class_name)
        } else {
            false
        }
    }

    pub async fn exception(&self, r#type: &str, message: &str) -> JavaError {
        tracing::info!("throwing java exception: {} {message}", r#type);

        let message_str = JavaLangString::from_rust_string(self, message).await.unwrap();
        let instance = self.new_class(r#type, "(Ljava/lang/String;)V", (message_str,)).await.unwrap();

        JavaError::JavaException(instance)
    }

    pub fn stack_trace(&self) -> Vec<String> {
        // TODO we should return in another format

        let thread_id = (self.inner.get_current_thread_id)();
        let threads = self.inner.threads.read();
        let thread = threads.get(&thread_id).unwrap();

        thread
            .iter_java_frame()
            .rev()
            .filter_map(|x| {
                // skip exception classes
                if self.is_inherited_from(&*x.class.definition, "java/lang/Throwable") {
                    None
                } else {
                    Some(format!("{}.{}", x.class.definition.name(), x.method))
                }
            })
            .collect()
    }

    /// Install the backend's conservative root scanner. See [`ConservativeRootScanner`].
    ///
    /// Set once, at backend start-up. All three `collect_garbage` call sites go through it,
    /// which is the point: `System.gc()` and `Runtime.gc()` are generic code with no way to
    /// reach an ARM core, and hero2 calls `System.gc()` itself.
    pub fn set_conservative_root_scanner(&self, scanner: ConservativeRootScanner) {
        *self.inner.conservative_root_scanner.write() = Some(scanner);
    }

    pub fn collect_garbage(&self) -> Result<usize> {
        tracing::trace!("Collecting garbage");

        // Held across the whole collection, mark and sweep alike: both read object fields, and on
        // an emulated-memory backend both can be the first to touch a dangling reference.
        let _collecting = CollectingGuard::enter();

        // ★ Taken BEFORE `all_objects` is locked and the walk starts, because the scanner
        // reads the guest's own machine state and must not be holding a collector lock
        // while it does. The set is small: only 4-aligned words inside the backend's object
        // heap survive its own filter.
        let conservative: HashSet<u64> = {
            let scanner = self.inner.conservative_root_scanner.read();
            match scanner.as_ref() {
                Some(scan) => {
                    let mut out = HashSet::new();
                    scan(&mut |address| {
                        out.insert(address);
                    });
                    out
                }
                None => HashSet::new(),
            }
        };

        let garbage = {
            let threads = self.inner.threads.read();
            let all_objects = self.inner.all_objects.read();
            let classes = self.inner.classes.read();
            let interned_strings = self.interned_strings();
            let pinned = self.pinned_objects();

            determine_garbage(self, &threads, &all_objects, &classes, &interned_strings, &pinned, &conservative)
        };

        let garbage_count = garbage.len();

        tracing::trace!("Garbage count: {garbage_count}");

        for object in garbage {
            // The class name used to be resolved HERE, for every object freed, whether or
            // not anything was listening: on the KTF backend that is two reads out of
            // emulated guest memory plus a `String`, per corpse, per frame. Inside the
            // macro it costs nothing until someone turns the level up (2026-09-18).
            tracing::trace!("Destroying {object:?}({})", object.class_definition().name());

            self.destroy(object).unwrap();
        }

        Ok(garbage_count)
    }

    async fn register_class_internal(&self, class: Class, class_loader_wrapper: Option<&dyn ClassLoaderWrapper>) -> Result<()> {
        if !class.definition.name().starts_with('[') {
            // ensure superclass and superinterfaces are loaded
            if let Some(super_class) = class.definition.super_class_name()
                && !self.has_class(&super_class)
            {
                self.resolve_class_internal(&super_class, class_loader_wrapper).await?;
            }

            for interface in class.definition.interface_names() {
                if !self.has_class(&interface) {
                    self.resolve_class_internal(&interface, class_loader_wrapper).await?;
                }
            }
        }

        self.inner.classes.write().insert(class.definition.name().to_owned(), class.clone());

        Ok(())
    }

    #[async_recursion::async_recursion]
    pub async fn ensure_initialized(&self, class: &Class) -> Result<()> {
        // ★ heroZero fork (2026-10-27): an initialized class first - every static call and static
        // field access comes here, and the array test below builds the class's name as a String.
        if class.init_state() == InitState::Initialized {
            return Ok(());
        }
        if class.definition.name().starts_with('[') {
            return Ok(());
        }

        match class.init_state() {
            InitState::Initialized | InitState::InProgress => return Ok(()),
            InitState::Erroneous => {
                return Err(self
                    .exception(
                        "java/lang/NoClassDefFoundError",
                        &format!("Could not initialize class {}", class.definition.name()),
                    )
                    .await);
            }
            InitState::NotInitialized => {}
        }

        class.set_init_state(InitState::InProgress);

        if let Some(super_name) = class.definition.super_class_name() {
            // resolution failure is not an initialization failure, so initialization may be retried
            let super_class = match self.resolve_class(&super_name).await {
                Ok(x) => x,
                Err(err) => {
                    class.set_init_state(InitState::NotInitialized);
                    return Err(err);
                }
            };

            if let Err(err) = self.ensure_initialized(&super_class).await {
                class.set_init_state(InitState::Erroneous);
                return Err(err);
            }
        }

        if let Err(err) = class.definition.prepare(self).await {
            class.set_init_state(InitState::Erroneous);
            return Err(err);
        }

        if let Some(clinit) = class.definition.method("<clinit>", "()V", true) {
            tracing::debug!("Calling <clinit> for {}", class.definition.name());

            if let Err(err) = self.execute_method(class, None, &*clinit, Box::new([])).await {
                class.set_init_state(InitState::Erroneous);

                let JavaError::JavaException(exception) = &err;
                if self.is_instance(&**exception, "java/lang/Error") {
                    return Err(err);
                }

                let cause = clone_box(&**exception);
                let wrapped = self
                    .new_class("java/lang/ExceptionInInitializerError", "(Ljava/lang/Throwable;)V", (cause,))
                    .await?;

                return Err(JavaError::JavaException(wrapped));
            }
        }

        class.set_init_state(InitState::Initialized);

        Ok(())
    }

    pub fn attach_thread(&self) -> Result<()> {
        let thread_id = (self.inner.get_current_thread_id)();
        self.inner.threads.write().insert(thread_id, JvmThread::new());
        self.push_native_frame();

        Ok(())
    }

    pub fn detach_thread(&self) -> Result<()> {
        let thread_id = (self.inner.get_current_thread_id)();
        self.inner.threads.write().remove(&thread_id);

        Ok(())
    }

    // TODO we need safe, ergonomic api..
    pub fn push_native_frame(&self) {
        let thread_id = (self.inner.get_current_thread_id)();
        self.inner.threads.write().get_mut(&thread_id).unwrap().push_native_frame();
    }

    pub fn pop_frame(&self) {
        let thread_id = (self.inner.get_current_thread_id)();
        self.inner.threads.write().get_mut(&thread_id).unwrap().pop_frame();
    }

    pub async fn current_class_loader(&self) -> Result<Box<dyn ClassInstance>> {
        let calling_class = self.find_calling_class()?;

        if let Some((class, class_instance)) = calling_class {
            // called in java

            if self.is_inherited_from(&*class.definition, "java/lang/ClassLoader") {
                return Ok(class_instance.unwrap());
            }

            let calling_class_class_loader = JavaLangClass::class_loader(self, &class.java_class()).await?;
            if let Some(x) = calling_class_class_loader {
                Ok(x)
            } else {
                let system_class_loader = JavaLangClassLoader::get_system_class_loader(self).await?;
                Ok(system_class_loader)
            }
        } else {
            // called outside of java

            let system_class_loader = JavaLangClassLoader::get_system_class_loader(self).await?;
            Ok(system_class_loader)
        }
    }

    fn get_or_create_monitor(&self, obj: &Box<dyn ClassInstance>) -> Arc<Event> {
        let key = self.inner.monitor_hasher.hash_one(obj);

        let monitors = self.inner.monitors.read();
        if let Some(monitor) = monitors.get(&key) {
            return monitor.clone();
        }
        drop(monitors);

        let mut monitors = self.inner.monitors.write();
        monitors.entry(key).or_insert_with(|| Arc::new(Event::new())).clone()
    }

    // JVMS 5.4.3.2 field resolution: search the class, then its superinterfaces, then its superclass.
    // Matching is by name and descriptor only; the caller checks static-ness afterwards.
    fn resolve_field(&self, class: &Class, name: &str, descriptor: &str) -> Option<(Class, Box<dyn Field>)> {
        if let Some(field) = class
            .definition
            .field(name, descriptor, true)
            .or_else(|| class.definition.field(name, descriptor, false))
        {
            return Some((class.clone(), field));
        }

        for interface in class.definition.interface_names() {
            if let Some(interface_class) = self.get_class(&interface)
                && let Some(found) = self.resolve_field(&interface_class, name, descriptor)
            {
                return Some(found);
            }
        }

        let super_class = self.get_class(&class.definition.super_class_name()?)?;
        self.resolve_field(&super_class, name, descriptor)
    }

    // JVMS 5.4.3.3 method resolution: search the class, then its superclass (interfaces have no static methods
    // in a 1.2 target). Matching is by name and descriptor only; the caller checks static-ness afterwards.
    fn resolve_method(&self, class: &Class, name: &str, descriptor: &str) -> Option<(Class, Box<dyn Method>)> {
        if let Some(method) = class
            .definition
            .method(name, descriptor, true)
            .or_else(|| class.definition.method(name, descriptor, false))
        {
            return Some((class.clone(), method));
        }

        let super_class = self.get_class(&class.definition.super_class_name()?)?;
        self.resolve_method(&super_class, name, descriptor)
    }

    /// ★ heroZero fork: an instance method for `invokespecial`, searched from `class` up its
    /// superclasses. Returns the declaring class with the method. See `invoke_special`.
    fn resolve_special_method(&self, class: &Class, name: &str, descriptor: &str) -> Option<(Class, Box<dyn Method>)> {
        if let Some(method) = class.definition.method(name, descriptor, false) {
            return Some((class.clone(), method));
        }

        let super_class = self.get_class(&class.definition.super_class_name()?)?;
        self.resolve_special_method(&super_class, name, descriptor)
    }

    pub fn find_field(&self, class: &dyn ClassDefinition, name: &str, descriptor: &str) -> Result<Option<Box<dyn Field>>> {
        let field = class.field(name, descriptor, false);

        if let Some(x) = field {
            Ok(Some(x))
        } else if let Some(x) = class.super_class_name() {
            let super_class = self.inner.classes.read().get(&x).unwrap().definition.clone();
            self.find_field(&*super_class, name, descriptor)
        } else {
            Ok(None)
        }
    }

    fn find_virtual_method(&self, class: &dyn ClassDefinition, name: &str, descriptor: &str, is_static: bool) -> Result<Option<Box<dyn Method>>> {
        let method = class.method(name, descriptor, false);

        if let Some(x) = method {
            if x.access_flags().contains(MethodAccessFlags::STATIC) == is_static {
                return Ok(Some(x));
            }
        } else if let Some(x) = class.super_class_name() {
            let super_class = self.inner.classes.read().get(&x).unwrap().definition.clone();
            return self.find_virtual_method(&*super_class, name, descriptor, is_static);
        }

        Ok(None)
    }

    async fn execute_method(
        &self,
        class: &Class,
        class_instance: Option<Box<dyn ClassInstance>>,
        method: &dyn Method,
        args: Box<[JavaValue]>,
    ) -> Result<JavaValue> {
        let thread_id = (self.inner.get_current_thread_id)();

        {
            let mut threads = self.inner.threads.write();
            let thread = threads.get_mut(&thread_id).unwrap();
            thread.push_java_frame(class, class_instance, method.signature());

            // ★ Arguments are roots for the length of the call, and this frame is the only place
            // that can hold them: the caller passes them by value into `method.run` below, so once
            // it has, nothing the collector scans refers to them. A reference that reaches a
            // method ONLY as an argument was therefore collectable while the method was running.
            // On the KTF backend that means the callee's own parameter is freed guest memory.
            let frame = thread.top_frame_mut();
            for arg in &args {
                if let JavaValue::Object(Some(x)) = arg {
                    frame.local_variables_mut().push(x.clone());
                }
            }
        }

        let result = method.run(self, args).await;

        tracing::trace!("Execute result: {result:?}");

        self.inner.threads.write().get_mut(&thread_id).unwrap().pop_frame();

        result
    }
}
