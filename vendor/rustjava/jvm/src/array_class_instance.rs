use alloc::{boxed::Box, vec::Vec};

use crate::{JavaType, Result, class_definition::ClassDefinition, class_instance::ClassInstance, field::Field, value::JavaValue};

#[async_trait::async_trait]
pub trait ArrayClassInstance: ClassInstance {
    fn class_definition(&self) -> Box<dyn ClassDefinition>;
    fn destroy(self: Box<Self>);
    fn equals(&self, other: &dyn ClassInstance) -> Result<bool>;
    fn store(&mut self, offset: usize, values: Box<[JavaValue]>) -> Result<()>;
    fn load(&self, offset: usize, count: usize) -> Result<Vec<JavaValue>>;
    fn raw_buffer(&self) -> Result<Box<dyn ArrayRawBuffer>>;
    fn raw_buffer_mut(&mut self) -> Result<Box<dyn ArrayRawBufferMut>>;
    fn length(&self) -> usize;

    /// ★ heroZero fork (2026-10-27): one element, for the interpreter's `xaload` / `xastore` -
    /// `load` and `store` take and give a `Vec`, two allocations an element access. The defaults
    /// go through them; a backend that holds its elements overrides these. The index is checked
    /// by the caller.
    fn load_one(&self, index: usize) -> Result<JavaValue> {
        Ok(self.load(index, 1)?.pop().unwrap())
    }

    fn store_one(&mut self, index: usize, value: JavaValue) -> Result<()> {
        self.store(index, Box::new([value]))
    }

    /// The element type, if the backend keeps it parsed (`Jvm::array_element_type` parses the
    /// class name otherwise).
    fn element_type_ref(&self) -> Option<&JavaType> {
        None
    }

    /// See [`ClassInstance::gc_identity`]; the blanket impl below forwards to this.
    fn gc_identity(&self) -> u64 {
        crate::class_instance::dyn_hash_identity(self)
    }

    /// See [`ClassInstance::gc_visit_references`]; the blanket impl below forwards to this.
    fn gc_visit_references(&self, _visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        false
    }

    /// See [`ClassInstance::gc_raw_address`]; the blanket impl below forwards to this.
    ///
    /// ⛔ FORWARDED, like the two above (2026-09-20, review of the conservative scan).
    /// The blanket `impl ClassInstance for T` below shadows EVERY default `ClassInstance`
    /// declares, so a hook that is not forwarded here is one an array can never answer:
    /// it falls back to `None` and no array is ever rescued by the conservative root scan,
    /// however exactly the guest's registers name it. Arrays are most of what AOT guest
    /// code keeps in a register (`byte[]` buffers, the `[[B` map grids, image data), so
    /// that hole is the same use-after-free the scan was added to close.
    fn gc_raw_address(&self) -> Option<u64> {
        None
    }
}

#[async_trait::async_trait]
impl<T: ArrayClassInstance> ClassInstance for T {
    fn destroy(self: Box<Self>) {
        ArrayClassInstance::destroy(self)
    }

    fn class_definition(&self) -> Box<dyn ClassDefinition> {
        ArrayClassInstance::class_definition(self)
    }

    fn equals(&self, other: &dyn ClassInstance) -> Result<bool> {
        ArrayClassInstance::equals(self, other)
    }
    fn as_array_instance(&self) -> Option<&dyn ArrayClassInstance> {
        Some(self)
    }

    fn as_array_instance_mut(&mut self) -> Option<&mut dyn ArrayClassInstance> {
        Some(self)
    }

    fn get_field(&self, _field: &dyn Field) -> Result<JavaValue> {
        panic!("Array classes do not have fields")
    }

    fn put_field(&mut self, _field: &dyn Field, _value: JavaValue) -> Result<()> {
        panic!("Array classes do not have fields")
    }

    fn gc_identity(&self) -> u64 {
        ArrayClassInstance::gc_identity(self)
    }

    fn gc_visit_references(&self, visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        ArrayClassInstance::gc_visit_references(self, visit)
    }

    fn gc_raw_address(&self) -> Option<u64> {
        ArrayClassInstance::gc_raw_address(self)
    }
}

pub trait ArrayRawBuffer: Send {
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<()>;
}

pub trait ArrayRawBufferMut: ArrayRawBuffer {
    fn write(&mut self, offset: usize, buffer: &[u8]) -> Result<()>;
}
