use alloc::{boxed::Box, sync::Arc, vec, vec::Vec};
use core::{
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
};

use parking_lot::RwLock;

use jvm::{ArrayClassDefinition, ArrayClassInstance, ArrayRawBuffer, ArrayRawBufferMut, ClassDefinition, ClassInstance, JavaType, JavaValue, Result};

use crate::array_class_definition::ArrayClassDefinitionImpl;

enum ArrayElements {
    Primitive(Vec<u8>),
    NonPrimitive(Vec<JavaValue>),
}

struct ArrayClassInstanceInner {
    class: Box<dyn ClassDefinition>,
    length: usize,
    element_type: JavaType,
    elements: RwLock<ArrayElements>,
}

#[derive(Clone)]
pub struct ArrayClassInstanceImpl {
    inner: Arc<ArrayClassInstanceInner>,
}

impl ArrayClassInstanceImpl {
    pub fn new(class: &ArrayClassDefinitionImpl, length: usize) -> Self {
        let element_type = JavaType::parse(&class.element_type_name());

        let elements = if matches!(element_type, JavaType::Class(_) | JavaType::Array(_)) {
            let default_value = element_type.default();
            ArrayElements::NonPrimitive(vec![default_value; length])
        } else {
            let element_size = Self::primitive_element_size(&element_type);
            ArrayElements::Primitive(vec![0; length * element_size])
        };

        Self {
            inner: Arc::new(ArrayClassInstanceInner {
                class: Box::new(class.clone()),
                length,
                element_type,
                elements: RwLock::new(elements),
            }),
        }
    }

    fn primitive_element_size(element_type: &JavaType) -> usize {
        match element_type {
            JavaType::Boolean => 1,
            JavaType::Byte => 1,
            JavaType::Char => 2,
            JavaType::Short => 2,
            JavaType::Int => 4,
            JavaType::Long => 8,
            JavaType::Float => 4,
            JavaType::Double => 8,
            _ => unreachable!(),
        }
    }

    fn convert_values_to_primitive(&self, values: Box<[JavaValue]>) -> Vec<u8> {
        match self.inner.element_type {
            JavaType::Boolean => values.into_vec().into_iter().map(|x| bool::from(x) as u8).collect::<Vec<_>>(),
            JavaType::Byte => values.into_vec().into_iter().map(|x| i8::from(x) as u8).collect::<Vec<_>>(),
            JavaType::Char => values.into_vec().into_iter().flat_map(|x| u16::from(x).to_le_bytes()).collect::<Vec<_>>(),
            JavaType::Short => values.into_vec().into_iter().flat_map(|x| i16::from(x).to_le_bytes()).collect::<Vec<_>>(),
            JavaType::Int => values.into_vec().into_iter().flat_map(|x| i32::from(x).to_le_bytes()).collect::<Vec<_>>(),
            JavaType::Long => values.into_vec().into_iter().flat_map(|x| i64::from(x).to_le_bytes()).collect::<Vec<_>>(),
            JavaType::Float => values.into_vec().into_iter().flat_map(|x| f32::from(x).to_le_bytes()).collect::<Vec<_>>(),
            JavaType::Double => values.into_vec().into_iter().flat_map(|x| f64::from(x).to_le_bytes()).collect::<Vec<_>>(),
            _ => unreachable!(),
        }
    }

    fn convert_primitive_to_values(&self, values_raw: &[u8]) -> Vec<JavaValue> {
        match self.inner.element_type {
            JavaType::Boolean => values_raw.iter().map(|&x| JavaValue::Boolean(x != 0)).collect::<Vec<_>>(),
            JavaType::Byte => values_raw.iter().map(|&x| JavaValue::Byte(x as i8)).collect::<Vec<_>>(),
            JavaType::Char => values_raw
                .chunks(2)
                .map(|x| JavaValue::Char(u16::from_le_bytes(x.try_into().unwrap())))
                .collect::<Vec<_>>(),
            JavaType::Short => values_raw
                .chunks(2)
                .map(|x| JavaValue::Short(i16::from_le_bytes(x.try_into().unwrap())))
                .collect::<Vec<_>>(),
            JavaType::Int => values_raw
                .chunks(4)
                .map(|x| JavaValue::Int(i32::from_le_bytes(x.try_into().unwrap())))
                .collect::<Vec<_>>(),
            JavaType::Long => values_raw
                .chunks(8)
                .map(|x| JavaValue::Long(i64::from_le_bytes(x.try_into().unwrap())))
                .collect::<Vec<_>>(),
            JavaType::Float => values_raw
                .chunks(4)
                .map(|x| JavaValue::Float(f32::from_le_bytes(x.try_into().unwrap())))
                .collect::<Vec<_>>(),
            JavaType::Double => values_raw
                .chunks(8)
                .map(|x| JavaValue::Double(f64::from_le_bytes(x.try_into().unwrap())))
                .collect::<Vec<_>>(),
            _ => unreachable!(),
        }
    }
}

#[async_trait::async_trait]
impl ArrayClassInstance for ArrayClassInstanceImpl {
    fn class_definition(&self) -> Box<dyn ClassDefinition> {
        self.inner.class.clone()
    }

    fn destroy(self: Box<Self>) {}

    fn equals(&self, other: &dyn ClassInstance) -> Result<bool> {
        let other = other.as_any().downcast_ref::<ArrayClassInstanceImpl>();
        if other.is_none() {
            return Ok(false);
        }
        let other = other.unwrap();

        Ok(Arc::ptr_eq(&self.inner, &other.inner))
    }

    fn store(&mut self, offset: usize, values: Box<[JavaValue]>) -> Result<()> {
        match &mut *self.inner.elements.write() {
            ArrayElements::Primitive(x) => {
                let element_size = Self::primitive_element_size(&self.inner.element_type);
                let values_raw = self.convert_values_to_primitive(values);

                x.splice(offset * element_size..offset * element_size + values_raw.len(), values_raw);
            }
            ArrayElements::NonPrimitive(x) => {
                x.splice(offset..offset + values.len(), values.into_vec());
            }
        }

        Ok(())
    }

    fn load(&self, offset: usize, length: usize) -> Result<Vec<JavaValue>> {
        Ok(match &*self.inner.elements.read() {
            ArrayElements::Primitive(x) => {
                let element_size = Self::primitive_element_size(&self.inner.element_type);
                let values_raw = &x[offset * element_size..offset * element_size + length * element_size];

                self.convert_primitive_to_values(values_raw)
            }
            ArrayElements::NonPrimitive(x) => x[offset..offset + length].to_vec(),
        })
    }

    fn raw_buffer(&self) -> Result<Box<dyn ArrayRawBuffer>> {
        Ok(Box::new(ArrayRawBufferImpl { inner: self.inner.clone() }))
    }

    fn raw_buffer_mut(&mut self) -> Result<Box<dyn ArrayRawBufferMut>> {
        Ok(Box::new(ArrayRawBufferImpl { inner: self.inner.clone() }))
    }

    fn length(&self) -> usize {
        self.inner.length
    }

    // ★ heroZero fork (2026-10-27): the element in place, no `Vec` either way.
    fn load_one(&self, index: usize) -> Result<JavaValue> {
        Ok(match &*self.inner.elements.read() {
            ArrayElements::Primitive(x) => {
                let size = Self::primitive_element_size(&self.inner.element_type);
                let raw = &x[index * size..(index + 1) * size];
                match self.inner.element_type {
                    JavaType::Boolean => JavaValue::Boolean(raw[0] != 0),
                    JavaType::Byte => JavaValue::Byte(raw[0] as i8),
                    JavaType::Char => JavaValue::Char(u16::from_le_bytes([raw[0], raw[1]])),
                    JavaType::Short => JavaValue::Short(i16::from_le_bytes([raw[0], raw[1]])),
                    JavaType::Int => JavaValue::Int(i32::from_le_bytes(raw.try_into().unwrap())),
                    JavaType::Long => JavaValue::Long(i64::from_le_bytes(raw.try_into().unwrap())),
                    JavaType::Float => JavaValue::Float(f32::from_le_bytes(raw.try_into().unwrap())),
                    JavaType::Double => JavaValue::Double(f64::from_le_bytes(raw.try_into().unwrap())),
                    _ => unreachable!(),
                }
            }
            ArrayElements::NonPrimitive(x) => x[index].clone(),
        })
    }

    fn store_one(&mut self, index: usize, value: JavaValue) -> Result<()> {
        match &mut *self.inner.elements.write() {
            ArrayElements::Primitive(x) => {
                let size = Self::primitive_element_size(&self.inner.element_type);
                let at = &mut x[index * size..(index + 1) * size];
                match self.inner.element_type {
                    JavaType::Boolean => at[0] = bool::from(value) as u8,
                    JavaType::Byte => at[0] = i8::from(value) as u8,
                    JavaType::Char => at.copy_from_slice(&u16::from(value).to_le_bytes()),
                    JavaType::Short => at.copy_from_slice(&i16::from(value).to_le_bytes()),
                    JavaType::Int => at.copy_from_slice(&i32::from(value).to_le_bytes()),
                    JavaType::Long => at.copy_from_slice(&i64::from(value).to_le_bytes()),
                    JavaType::Float => at.copy_from_slice(&f32::from(value).to_le_bytes()),
                    JavaType::Double => at.copy_from_slice(&f64::from(value).to_le_bytes()),
                    _ => unreachable!(),
                }
            }
            ArrayElements::NonPrimitive(x) => x[index] = value,
        }

        Ok(())
    }

    fn element_type_ref(&self) -> Option<&JavaType> {
        Some(&self.inner.element_type)
    }

    // ★ heroZero fork (2026-10-27): the collector's walk over the elements in place - the generic
    // one loads a copy of the whole array (a `Box` clone per element) for every object array, on
    // every collection.
    fn gc_visit_references(&self, visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        if let ArrayElements::NonPrimitive(x) = &*self.inner.elements.read() {
            for value in x {
                if let JavaValue::Object(Some(object)) = value {
                    visit(&**object);
                }
            }
        }
        true
    }
}

impl Hash for ArrayClassInstanceImpl {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.inner).hash(state);
    }
}

impl Debug for ArrayClassInstanceImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ArrayClassInstance({})", self.inner.class.name())
    }
}

struct ArrayRawBufferImpl {
    inner: Arc<ArrayClassInstanceInner>,
}

impl ArrayRawBuffer for ArrayRawBufferImpl {
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<()> {
        match &*self.inner.elements.read() {
            ArrayElements::Primitive(x) => {
                let element_size = ArrayClassInstanceImpl::primitive_element_size(&self.inner.element_type);
                let values_raw = &x[offset * element_size..offset * element_size + buffer.len() * element_size];

                buffer.copy_from_slice(values_raw);
            }
            ArrayElements::NonPrimitive(_) => {
                panic!("Expected primitive array");
            }
        }

        Ok(())
    }
}

impl ArrayRawBufferMut for ArrayRawBufferImpl {
    fn write(&mut self, offset: usize, buffer: &[u8]) -> Result<()> {
        if let ArrayElements::Primitive(x) = &mut *self.inner.elements.write() {
            let element_size = ArrayClassInstanceImpl::primitive_element_size(&self.inner.element_type);

            x[offset * element_size..offset * element_size + buffer.len() * element_size].copy_from_slice(buffer);
        } else {
            panic!("Expected primitive array");
        }

        Ok(())
    }
}
