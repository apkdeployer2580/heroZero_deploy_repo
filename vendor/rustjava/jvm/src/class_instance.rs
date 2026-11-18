use alloc::boxed::Box;
use core::{
    fmt::{self, Debug, Formatter},
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use dyn_clone::{DynClone, clone_trait_object};
use dyn_hash::{DynHash, hash_trait_object};

use crate::{ArrayClassInstance, ClassDefinition, Field, Jvm, Result, as_any::AsAny, value::JavaValue};

#[async_trait::async_trait]
pub trait ClassInstance: Sync + Send + AsAny + Debug + DynHash + DynClone + 'static {
    fn destroy(self: Box<Self>);
    fn class_definition(&self) -> Box<dyn ClassDefinition>;
    fn equals(&self, other: &dyn ClassInstance) -> Result<bool>;
    fn get_field(&self, field: &dyn Field) -> Result<JavaValue>;
    fn put_field(&mut self, field: &dyn Field, value: JavaValue) -> Result<()>;
    fn as_array_instance(&self) -> Option<&dyn ArrayClassInstance> {
        None
    }
    fn as_array_instance_mut(&mut self) -> Option<&mut dyn ArrayClassInstance> {
        None
    }

    /// Read `name:descriptor`, resolving the field through this object's class and its
    /// superclasses. `Ok(None)` means no such field. The default goes through
    /// [`class_definition`](Self::class_definition) and [`get_field`](Self::get_field) -
    /// two boxed trait objects per read. A backend that memoises its class metadata
    /// host-side (wie's KTF runtime) overrides this to answer without allocating; on hero2
    /// this is the single most frequent JVM operation (~1,200 a frame).
    fn get_field_by_name(&self, jvm: &Jvm, name: &str, descriptor: &str) -> Result<Option<JavaValue>> {
        match jvm.find_field(&*self.class_definition(), name, descriptor)? {
            Some(field) => Ok(Some(self.get_field(&*field)?)),
            None => Ok(None),
        }
    }

    /// Write `name:descriptor`; `Ok(false)` means no such field. See `get_field_by_name`.
    fn put_field_by_name(&mut self, jvm: &Jvm, name: &str, descriptor: &str, value: JavaValue) -> Result<bool> {
        match jvm.find_field(&*self.class_definition(), name, descriptor)? {
            Some(field) => {
                self.put_field(&*field, value)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// The collector's identity for this object: equal for two handles to the same object,
    /// and (for a backend that overrides it) distinct otherwise. The default hashes the
    /// handle, which every backend already defines as the object's address - so the only
    /// failure mode of the default is a 64-bit collision, and that direction is a leak, not
    /// a freed live object (a colliding garbage object is kept). Lets the mark set be a
    /// `HashSet<u64>` instead of a set of boxed clones.
    fn gc_identity(&self) -> u64 {
        dyn_hash_identity(self)
    }

    /// The mark walk's fast path: hand every object this one references (its
    /// reference-typed instance fields, or its elements if it is an object array) to
    /// `visit`, and return `true`. Return `false` to have the collector do it the generic
    /// way, through `class_definition()`, `fields()` and `get_field()`. A backend whose
    /// objects live in memory it can read directly (wie's KTF runtime) overrides this so a
    /// collection of ~8,000 live objects allocates nothing per object.
    fn gc_visit_references(&self, _visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        false
    }

    /// ★ heroZero fork (2026-10-27): a number that names this object's CLASS for as long as the
    /// class lives (its definition's address) - the key `Jvm`'s resolution memo files a call or a
    /// field read on this object under. `None` (the default): no memo, every call on the object
    /// resolves the long way, as before.
    fn class_identity(&self) -> Option<usize> {
        None
    }

    /// The address this object lives at in the backend's own memory, if it has one.
    ///
    /// Only the CONSERVATIVE root scan uses it (see `Jvm::set_conservative_root_scanner`):
    /// a backend that runs AOT-compiled guest code cannot tell the collector which objects
    /// its registers and stacks are holding, so instead it hands over the raw words it
    /// found and the collector matches them against this.
    ///
    /// ⛔ It is NOT an identity — two objects may share `gc_identity` by collision but must
    /// never share an address. `None` (the default) simply opts a backend out of the scan.
    fn gc_raw_address(&self) -> Option<u64> {
        None
    }
}

/// The default `gc_identity`: the handle's own `Hash` through a FIXED hasher.
///
/// ⛔ Not `hashbrown::DefaultHashBuilder` - that is a randomly seeded `foldhash`, and every
/// `default()` seeds differently, so the same object hashed in the mark phase and in the
/// sweep would not match and everything would look like garbage. FNV-1a, 64-bit: stable,
/// dependency-free, and the input is a pointer's worth of bytes.
pub(crate) fn dyn_hash_identity<T: ?Sized + DynHash>(instance: &T) -> u64 {
    use core::hash::Hasher;

    struct Fnv1a(u64);

    impl Hasher for Fnv1a {
        fn finish(&self) -> u64 {
            self.0
        }

        fn write(&mut self, bytes: &[u8]) {
            for b in bytes {
                self.0 ^= *b as u64;
                self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }

    let mut hasher = Fnv1a(0xcbf2_9ce4_8422_2325);
    instance.dyn_hash(&mut hasher);
    hasher.finish()
}

clone_trait_object!(ClassInstance);
hash_trait_object!(ClassInstance);

impl Eq for dyn ClassInstance {}
impl PartialEq for dyn ClassInstance {
    fn eq(&self, other: &Self) -> bool {
        self.equals(other).unwrap()
    }
}

// array wrapper for ClassInstanceRef
pub struct Array<T>(PhantomData<T>);

// typesafe wrapper for ClassInstance
pub struct ClassInstanceRef<T> {
    pub instance: Option<Box<dyn ClassInstance>>,
    _phantom: PhantomData<T>,
}

impl<T> ClassInstanceRef<T> {
    pub fn new(instance: Option<Box<dyn ClassInstance>>) -> Self {
        Self {
            instance,
            _phantom: PhantomData,
        }
    }
}

impl<T> Clone for ClassInstanceRef<T> {
    fn clone(&self) -> Self {
        Self {
            instance: self.instance.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T> ClassInstanceRef<T> {
    pub fn is_null(&self) -> bool {
        self.instance.is_none()
    }
}

impl<T> Debug for ClassInstanceRef<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some(x) = &self.instance {
            write!(f, "{x:?}")
        } else {
            write!(f, "null")
        }
    }
}

impl<T> Deref for ClassInstanceRef<T> {
    type Target = Box<dyn ClassInstance>;
    fn deref(&self) -> &Self::Target {
        self.instance.as_ref().unwrap()
    }
}

impl<T> DerefMut for ClassInstanceRef<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.instance.as_mut().unwrap()
    }
}

impl<T> From<ClassInstanceRef<T>> for JavaValue {
    fn from(value: ClassInstanceRef<T>) -> Self {
        value.instance.into()
    }
}

impl<T> From<Box<dyn ClassInstance>> for ClassInstanceRef<T> {
    fn from(value: Box<dyn ClassInstance>) -> Self {
        Self {
            instance: Some(value),
            _phantom: PhantomData,
        }
    }
}

impl<T> From<Option<Box<dyn ClassInstance>>> for ClassInstanceRef<T> {
    fn from(value: Option<Box<dyn ClassInstance>>) -> Self {
        Self {
            instance: value,
            _phantom: PhantomData,
        }
    }
}

impl<T> From<JavaValue> for ClassInstanceRef<T> {
    fn from(val: JavaValue) -> Self {
        ClassInstanceRef {
            instance: val.into(),
            _phantom: PhantomData,
        }
    }
}

impl<T> From<ClassInstanceRef<T>> for Box<dyn ClassInstance> {
    fn from(value: ClassInstanceRef<T>) -> Self {
        value.instance.unwrap()
    }
}

impl<T> From<ClassInstanceRef<T>> for Option<Box<dyn ClassInstance>> {
    fn from(value: ClassInstanceRef<T>) -> Self {
        value.instance
    }
}
