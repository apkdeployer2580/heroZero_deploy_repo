use alloc::{boxed::Box, vec, vec::Vec};
use core::{
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
};

use jvm::{ArrayClassInstance, ArrayRawBuffer, ArrayRawBufferMut, ClassDefinition, ClassInstance, JavaType, JavaValue, Result as JvmResult};

use wie_core_arm::ArmCore;
use wie_util::{ByteRead, ByteWrite, read_generic, write_generic};

use super::{Result, array_class_definition::JavaArrayClassDefinition, class_instance::JavaClassInstance, liveness, value::JavaValueExt};

#[derive(Clone)]
pub struct JavaArrayClassInstance {
    pub class_instance: JavaClassInstance,
    core: ArmCore,
}

impl JavaArrayClassInstance {
    pub fn from_raw(ptr_raw: u32, core: &ArmCore) -> Self {
        Self {
            class_instance: JavaClassInstance::from_raw(ptr_raw, core),
            core: core.clone(),
        }
    }

    pub fn new(core: &mut ArmCore, array_class: &JavaArrayClassDefinition, count: usize) -> Result<Self> {
        let element_size = array_class.element_size()?;
        let class_instance = JavaClassInstance::instantiate(core, &array_class.class, count * element_size + 4)?;

        let length_address = class_instance.field_address(0)?;
        write_generic(core, length_address, count as u32)?;

        Ok(Self::from_raw(class_instance.ptr_raw, core))
    }

    pub fn load_raw(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let base_address = self.base_address()?;

        self.core.read_bytes(base_address + offset as u32, buf)?;

        Ok(())
    }

    pub fn store_raw(&mut self, offset: usize, values_raw: Vec<u8>) -> Result<()> {
        let base_address = self.base_address()?;

        self.core.write_bytes(base_address + offset as u32, &values_raw)
    }

    pub fn array_length(&self) -> Result<usize> {
        let length_address = self.class_instance.field_address(0)?;
        let result: u32 = read_generic(&self.core, length_address)?;

        Ok(result as _)
    }

    fn element_size(&self) -> Result<usize> {
        let array_class = JavaArrayClassDefinition::from_raw(self.class_instance.class()?.ptr_raw, &self.core);

        array_class.element_size()
    }

    fn element_type(&self) -> Result<JavaType> {
        let array_class = JavaArrayClassDefinition::from_raw(self.class_instance.class()?.ptr_raw, &self.core);

        Ok(JavaType::parse(&array_class.element_type_descriptor()?))
    }

    fn base_address(&self) -> Result<u32> {
        self.class_instance.field_address(4)
    }

    /// A dead array yields a buffer over address 0, whose reads and writes are refused by the
    /// engine and swallowed below. Neither signature can carry an error - `JvmResult`'s only
    /// error is a java exception, which needs an async allocation to build.
    fn buffer_impl(&self) -> ArrayRawBufferImpl {
        let (Ok(base_address), Ok(element_size)) = (self.base_address(), self.element_size()) else {
            self.report("raw buffer");
            return ArrayRawBufferImpl {
                core: self.core.clone(),
                base_address: 0,
                element_size: 1,
            };
        };

        ArrayRawBufferImpl {
            core: self.core.clone(),
            base_address,
            element_size: element_size as _,
        }
    }

    /// Every refusal above routes through here, so no array fallback is ever silent. The
    /// liveness report already distinguishes "the collector freed it" from "it was never an
    /// instance"; this only has to say which array operation asked.
    fn report(&self, what: &str) {
        let ptr_raw = self.class_instance.ptr_raw;
        let class = self
            .class_instance
            .class()
            .and_then(|x| x.name())
            .unwrap_or_else(|_| alloc::string::String::from("<unreadable>"));

        liveness::report_refusal(what, ptr_raw, || alloc::format!("array class={class}"));
    }
}

#[async_trait::async_trait]
impl ArrayClassInstance for JavaArrayClassInstance {
    fn gc_identity(&self) -> u64 {
        self.class_instance.ptr_raw as u64
    }

    /// The array record's own guest address, for the conservative root scan. Same value
    /// as [`Self::gc_identity`]; see `JavaClassInstance::gc_raw_address` for why the two
    /// are separate methods.
    fn gc_raw_address(&self) -> Option<u64> {
        Some(self.class_instance.ptr_raw as u64)
    }

    fn gc_visit_references(&self, visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        // Same record, same walk: the instance side reads the class and finds it is an array.
        self.class_instance.gc_visit_references_raw(visit);

        true
    }

    fn destroy(self: Box<Self>) {
        // Sizing a dead array reads its own length out of the field block that is already gone,
        // so all three of these can fail. Leak rather than panic - see the same reasoning on
        // `JavaClassInstance::destroy`. (`field_address` has already reported the reason.)
        let size = self.element_size().and_then(|e| Ok(e * self.array_length()? + 4));
        let Ok(field_size) = size else {
            tracing::error!("Could not size array {:#x} to destroy it, leaking it", self.class_instance.ptr_raw);
            return;
        };

        if let Err(e) = self.class_instance.destroy(field_size as _) {
            tracing::error!("Could not destroy a java array, leaking it: {e:?}");
        }
    }

    fn class_definition(&self) -> Box<dyn ClassDefinition> {
        Box::new(self.class_instance.class().unwrap())
    }

    fn equals(&self, other: &dyn ClassInstance) -> JvmResult<bool> {
        let other = other.as_any().downcast_ref::<JavaArrayClassInstance>();
        if other.is_none() {
            return Ok(false);
        }

        Ok(self.class_instance.ptr_raw == other.unwrap().class_instance.ptr_raw)
    }

    fn store(&mut self, offset: usize, values: Box<[JavaValue]>) -> JvmResult<()> {
        let Ok(element_size) = self.element_size() else {
            self.report("store: element size");
            return Ok(());
        };

        let values = values.to_vec();

        let raw_values = match element_size {
            1 => values.into_iter().map(|x| x.as_raw() as u8).collect::<Vec<_>>(),
            2 => values
                .into_iter()
                .map(|x| x.as_raw() as u16)
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
            4 => values.into_iter().map(|x| x.as_raw()).flat_map(u32::to_le_bytes).collect::<Vec<_>>(),
            _ => unreachable!(),
        };

        let offset = offset * element_size;
        // A dead array drops the write, exactly as a dead instance drops `put_field`: the address
        // would land on the allocator's own free-chain bookkeeping.
        let _ = self.store_raw(offset as _, raw_values);

        Ok(())
    }

    fn load(&self, offset: usize, count: usize) -> JvmResult<Vec<JavaValue>> {
        // An empty read is what the collector's mark walk needs from a dead array: it iterates
        // whatever comes back looking for references, and no elements means no recursion into a
        // field block that the free chain now owns.
        let Ok(element_size) = self.element_size() else {
            self.report("load: element size");
            return Ok(Vec::new());
        };
        let offset = offset * element_size;

        let mut values_raw = vec![0; count * element_size];
        if self.load_raw(offset as _, &mut values_raw).is_err() {
            self.report("load: elements");
            return Ok(Vec::new());
        }

        let Ok(element_type) = self.element_type() else {
            self.report("load: element type");
            return Ok(Vec::new());
        };

        Ok(match element_size {
            1 => values_raw
                .into_iter()
                .map(|x| JavaValue::from_raw(x as _, &element_type, &self.core))
                .collect::<Vec<_>>(),
            2 => values_raw
                .chunks(2)
                .map(|x| JavaValue::from_raw(u16::from_le_bytes(x.try_into().unwrap()) as _, &element_type, &self.core))
                .collect::<Vec<_>>(),
            4 => values_raw
                .chunks(4)
                .map(|x| JavaValue::from_raw(u32::from_le_bytes(x.try_into().unwrap()) as _, &element_type, &self.core))
                .collect::<Vec<_>>(),
            _ => unreachable!(),
        })
    }

    fn raw_buffer(&self) -> JvmResult<Box<dyn ArrayRawBuffer>> {
        Ok(Box::new(self.buffer_impl()))
    }

    fn raw_buffer_mut(&mut self) -> JvmResult<Box<dyn ArrayRawBufferMut>> {
        Ok(Box::new(self.buffer_impl()))
    }

    /// Zero, not a panic, for a dead array. Callers use this to bound a loop, and the collector
    /// calls it as `load(0, length())` - so zero makes the walk skip the corpse quietly.
    fn length(&self) -> usize {
        match self.array_length() {
            Ok(x) => x,
            Err(_) => {
                self.report("length");
                0
            }
        }
    }
}

impl Debug for JavaArrayClassInstance {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.class_instance.ptr_raw)
    }
}

impl Hash for JavaArrayClassInstance {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.class_instance.hash(state);
    }
}

struct ArrayRawBufferImpl {
    core: ArmCore,
    base_address: u32,
    element_size: u32,
}

impl ArrayRawBuffer for ArrayRawBufferImpl {
    fn read(&self, offset: usize, buffer: &mut [u8]) -> JvmResult<()> {
        let address = self.base_address + (offset as u32) * self.element_size;
        if self.core.read_bytes(address, buffer).is_err() {
            tracing::error!("Read of {} bytes at {address:#x} from a dead java array refused", buffer.len());
        }

        Ok(())
    }
}

impl ArrayRawBufferMut for ArrayRawBufferImpl {
    fn write(&mut self, offset: usize, buffer: &[u8]) -> JvmResult<()> {
        let address = self.base_address + (offset as u32) * self.element_size;
        if self.core.write_bytes(address, buffer).is_err() {
            tracing::error!("Write of {} bytes at {address:#x} to a dead java array refused", buffer.len());
        }

        Ok(())
    }
}
