use alloc::{boxed::Box, string::String, vec::Vec};
use core::fmt::Debug;

use dyn_clone::{DynClone, clone_trait_object};

use java_constants::ClassAccessFlags;

use crate::{ArrayClassDefinition, ClassInstance, Field, JavaValue, Jvm, Method, Result, as_any::AsAny};

#[async_trait::async_trait]
pub trait ClassDefinition: Sync + Send + AsAny + Debug + DynClone {
    fn name(&self) -> String;
    fn super_class_name(&self) -> Option<String>;
    fn interface_names(&self) -> Vec<String>;
    fn access_flags(&self) -> ClassAccessFlags;
    async fn instantiate(&self, jvm: &Jvm) -> Result<Box<dyn ClassInstance>>;
    async fn prepare(&self, jvm: &Jvm) -> Result<()>;
    fn method(&self, name: &str, descriptor: &str, is_static: bool) -> Option<Box<dyn Method>>;
    fn field(&self, name: &str, descriptor: &str, is_static: bool) -> Option<Box<dyn Field>>;
    fn fields(&self) -> Vec<Box<dyn Field>>;
    fn get_static_field(&self, field: &dyn Field) -> Result<JavaValue>; // TODO do we need to split class? or rename classdefinition?
    /// `&self`: every backend stores statics behind shared state already (guest memory on
    /// wie, a lock on `jvm_rust`), and `&mut` here forced `Class` to hand out owned copies.
    fn put_static_field(&self, field: &dyn Field, value: JavaValue) -> Result<()>;
    fn as_array_class_definition(&self) -> Option<&dyn ArrayClassDefinition> {
        None
    }

    /// The mark walk's fast path for statics: hand every object a reference-typed static of
    /// THIS class (not its parents - the collector visits every class) refers to over to
    /// `visit`, and return `true`. `false` = let the collector enumerate the fields itself.
    /// See [`ClassInstance::gc_visit_references`](crate::ClassInstance::gc_visit_references).
    fn gc_visit_static_references(&self, _visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        false
    }
}

clone_trait_object!(ClassDefinition);
