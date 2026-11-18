use alloc::{boxed::Box, collections::BTreeMap, sync::Arc};
use core::{
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
};

use parking_lot::RwLock;

use jvm::{ClassDefinition, ClassInstance, Field, JavaType, JavaValue, Result};

use crate::{FieldImpl, class_definition::ClassDefinitionImpl};

struct ClassInstanceInner {
    class: Box<dyn ClassDefinition>,
    /// `ClassDefinitionImpl::identity` of `class`.
    class_identity: usize,
    storage: RwLock<BTreeMap<FieldImpl, JavaValue>>, // TODO we should use field offset or something
}

#[derive(Clone)]
pub struct ClassInstanceImpl {
    inner: Arc<ClassInstanceInner>,
}

impl ClassInstanceImpl {
    pub fn new(class: &ClassDefinitionImpl) -> Self {
        Self {
            inner: Arc::new(ClassInstanceInner {
                class: Box::new(class.clone()),
                class_identity: class.identity(),
                storage: RwLock::new(BTreeMap::new()),
            }),
        }
    }
}

#[async_trait::async_trait]
impl ClassInstance for ClassInstanceImpl {
    fn destroy(self: Box<Self>) {}

    fn class_definition(&self) -> Box<dyn ClassDefinition> {
        self.inner.class.clone()
    }

    fn class_identity(&self) -> Option<usize> {
        Some(self.inner.class_identity)
    }

    fn equals(&self, other: &dyn ClassInstance) -> Result<bool> {
        let other = other.as_any().downcast_ref::<ClassInstanceImpl>();
        if other.is_none() {
            return Ok(false);
        }
        let other = other.unwrap();

        Ok(Arc::ptr_eq(&self.inner, &other.inner))
    }

    fn get_field(&self, field: &dyn Field) -> Result<JavaValue> {
        let field = field.as_any().downcast_ref::<FieldImpl>().unwrap();

        let storage = self.inner.storage.read();
        let value = storage.get(field);

        if let Some(x) = value {
            Ok(x.clone())
        } else {
            Ok(JavaType::parse(&field.descriptor()).default())
        }
    }

    fn put_field(&mut self, field: &dyn Field, value: JavaValue) -> Result<()> {
        let field = field.as_any().downcast_ref::<FieldImpl>().unwrap();

        self.inner.storage.write().insert(field.clone(), value);

        Ok(())
    }

    // ★ heroZero fork (2026-10-27): the collector's walk over what this object holds, in place.
    // The storage has exactly the instance fields written so far, so its object values are the
    // references; the generic walk instead resolved every reference field of the class chain,
    // looked each up and cloned its value - allocations per field, per object, per collection.
    fn gc_visit_references(&self, visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        for value in self.inner.storage.read().values() {
            if let JavaValue::Object(Some(object)) = value {
                visit(&**object);
            }
        }
        true
    }
}

impl Hash for ClassInstanceImpl {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.inner).hash(state);
    }
}

impl Debug for ClassInstanceImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ClassInstance({})", self.inner.class.name())
    }
}
