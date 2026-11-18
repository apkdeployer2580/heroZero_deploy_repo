use alloc::{
    boxed::Box,
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::{
    fmt::{self, Debug, Formatter},
    ops::{Deref, DerefMut},
};

use parking_lot::RwLock;

use classfile::{AttributeInfo, ClassInfo, ConstantPoolReference};
use java_class_proto::JavaClassProto;
use java_constants::{ClassAccessFlags, FieldAccessFlags, MethodAccessFlags};
use jvm::{ClassDefinition, ClassInstance, Field, JavaType, JavaValue, Jvm, Method, Result};

use crate::{class_instance::ClassInstanceImpl, field::FieldImpl, method::MethodImpl};

struct ClassDefinitionInner {
    name: String,
    super_class_name: Option<String>,
    interfaces: Vec<String>,
    access_flags: ClassAccessFlags,
    methods: Vec<MethodImpl>,
    fields: Vec<FieldImpl>,
    constant_values: Vec<(FieldImpl, ConstantPoolReference)>,
    storage: RwLock<BTreeMap<FieldImpl, JavaValue>>, // TODO we should use field offset or something
}

#[derive(Clone)]
pub struct ClassDefinitionImpl {
    inner: Arc<ClassDefinitionInner>,
}

impl ClassDefinitionImpl {
    pub fn new(
        name: &str,
        super_class_name: Option<String>,
        interfaces: Vec<String>,
        access_flags: ClassAccessFlags,
        methods: Vec<MethodImpl>,
        fields: Vec<FieldImpl>,
    ) -> Self {
        Self::with_constant_values(name, super_class_name, interfaces, access_flags, methods, fields, Vec::new())
    }

    #[allow(clippy::too_many_arguments)]
    fn with_constant_values(
        name: &str,
        super_class_name: Option<String>,
        interfaces: Vec<String>,
        access_flags: ClassAccessFlags,
        methods: Vec<MethodImpl>,
        fields: Vec<FieldImpl>,
        constant_values: Vec<(FieldImpl, ConstantPoolReference)>,
    ) -> Self {
        Self {
            inner: Arc::new(ClassDefinitionInner {
                name: name.to_string(),
                super_class_name,
                interfaces,
                access_flags,
                methods,
                fields,
                constant_values,
                storage: RwLock::new(BTreeMap::new()),
            }),
        }
    }

    pub fn from_class_proto<C, Context>(proto: JavaClassProto<C>, context: Context) -> Self
    where
        C: ?Sized + 'static + Send,
        Context: Sync + Send + DerefMut + Deref<Target = C> + Clone + 'static,
    {
        let methods = proto
            .methods
            .into_iter()
            .map(|x| MethodImpl::from_method_proto(x, context.clone()))
            .collect::<Vec<_>>();

        let fields = proto.fields.into_iter().map(|x| FieldImpl::from_field_proto(proto.name, x)).collect::<Vec<_>>();

        let interfaces = proto.interfaces.into_iter().map(|x| x.to_string()).collect();

        Self::new(
            proto.name,
            proto.parent_class.map(|x| x.to_string()),
            interfaces,
            proto.access_flags,
            methods,
            fields,
        )
    }

    pub fn from_classfile(data: &[u8]) -> Result<Self> {
        let class = ClassInfo::parse(data).unwrap(); // TODO ClassFormatError
        assert_eq!(class.magic, 0xCAFEBABE);

        let mut constant_values = Vec::new();
        let fields = class
            .fields
            .into_iter()
            .map(|field_info| {
                let constant = field_info.attributes.iter().find_map(|x| match x {
                    AttributeInfo::ConstantValue(value) => Some(value.clone()),
                    _ => None,
                });

                let field = FieldImpl::from_field_info(&class.this_class, field_info);
                if let Some(x) = constant
                    && field.access_flags().contains(FieldAccessFlags::STATIC)
                {
                    constant_values.push((field.clone(), x));
                }

                field
            })
            .collect::<Vec<_>>();

        let methods = class.methods.into_iter().map(MethodImpl::from_method_info).collect::<Vec<_>>();

        let interfaces = class.interfaces.into_iter().map(|x| x.to_string()).collect();

        Ok(Self::with_constant_values(
            &class.this_class,
            class.super_class.map(|x| x.to_string()),
            interfaces,
            class.access_flags,
            methods,
            fields,
            constant_values,
        ))
    }

    pub fn fields(&self) -> &[FieldImpl] {
        &self.inner.fields
    }

    /// ★ heroZero fork (2026-10-27): the definition's address - the same for every clone of it,
    /// and alive as long as the class is (`ClassInstance::class_identity`).
    pub fn identity(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }
}

#[async_trait::async_trait]
impl ClassDefinition for ClassDefinitionImpl {
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    fn super_class_name(&self) -> Option<String> {
        self.inner.super_class_name.as_ref().map(|x| x.to_string())
    }

    fn interface_names(&self) -> Vec<String> {
        self.inner.interfaces.clone()
    }

    fn access_flags(&self) -> ClassAccessFlags {
        self.inner.access_flags
    }

    async fn instantiate(&self, _: &Jvm) -> Result<Box<dyn ClassInstance>> {
        Ok(Box::new(ClassInstanceImpl::new(self)))
    }

    async fn prepare(&self, jvm: &Jvm) -> Result<()> {
        for (field, constant) in &self.inner.constant_values {
            let value = match constant {
                ConstantPoolReference::Integer(x) => match field.descriptor().as_str() {
                    "Z" => JavaValue::Boolean(*x != 0),
                    "B" => JavaValue::Byte(*x as i8),
                    "C" => JavaValue::Char(*x as u16),
                    "S" => JavaValue::Short(*x as i16),
                    _ => JavaValue::Int(*x),
                },
                ConstantPoolReference::Long(x) => JavaValue::Long(*x),
                ConstantPoolReference::Float(x) => JavaValue::Float(*x),
                ConstantPoolReference::Double(x) => JavaValue::Double(*x),
                ConstantPoolReference::String(x) => JavaValue::Object(Some(jvm.intern_string(x).await?)),
                _ => continue,
            };

            self.inner.storage.write().insert(field.clone(), value);
        }

        Ok(())
    }

    // ★ heroZero fork (2026-10-27): compared in place (`name_str`), not through `name()` /
    // `descriptor()`, which copy - two allocations per candidate, on every call and field access.
    fn method(&self, name: &str, descriptor: &str, is_static: bool) -> Option<Box<dyn Method>> {
        self.inner
            .methods
            .iter()
            .find(|&method| {
                method.name_str() == name
                    && method.descriptor_str() == descriptor
                    && method.access_flags().contains(MethodAccessFlags::STATIC) == is_static
            })
            .map(|x| Box::new(x.clone()) as Box<dyn Method>)
    }

    fn field(&self, name: &str, descriptor: &str, is_static: bool) -> Option<Box<dyn Field>> {
        self.inner
            .fields
            .iter()
            .find(|&field| {
                field.name_str() == name && field.descriptor_str() == descriptor && field.access_flags().contains(FieldAccessFlags::STATIC) == is_static
            })
            .map(|x| Box::new(x.clone()) as Box<dyn Field>)
    }

    fn fields(&self) -> Vec<Box<dyn Field>> {
        self.inner.fields.iter().map(|x| Box::new(x.clone()) as Box<dyn Field>).collect()
    }

    fn get_static_field(&self, field: &dyn Field) -> Result<JavaValue> {
        let field = field.as_any().downcast_ref::<FieldImpl>().unwrap();

        let storage = self.inner.storage.read();
        let value = storage.get(field);

        if let Some(x) = value {
            Ok(x.clone())
        } else {
            Ok(JavaType::parse(&field.descriptor()).default())
        }
    }

    fn put_static_field(&self, field: &dyn Field, value: JavaValue) -> Result<()> {
        let field = field.as_any().downcast_ref::<FieldImpl>().unwrap();

        self.inner.storage.write().insert(field.clone(), value);

        Ok(())
    }

    // ★ heroZero fork (2026-10-27): this class's statics, in place (see `ClassInstanceImpl`). The
    // generic walk read the parents' statics through this class too and found nothing there - each
    // class's statics live in its own storage, and the collector visits every class.
    fn gc_visit_static_references(&self, visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        for value in self.inner.storage.read().values() {
            if let JavaValue::Object(Some(object)) = value {
                visit(&**object);
            }
        }
        true
    }
}

impl Debug for ClassDefinitionImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Class({})", self.name())
    }
}
