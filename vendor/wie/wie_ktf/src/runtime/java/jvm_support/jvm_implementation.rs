use alloc::{boxed::Box, collections::BTreeMap, format, sync::Arc};
use core::{
    ops::{Deref, DerefMut},
    pin::Pin,
};

use java_class_proto::JavaClassProto;
use jvm::{ClassDefinition, Jvm, Result as JvmResult};
use spin::Mutex;

use wie_core_arm::ArmCore;
use wie_jvm_support::JvmImplementation;
use wie_util::WieError;

use crate::runtime::java::{JavaSvcFunctions, register_java_svc_handler};

use super::{JavaArrayClassDefinition, JavaClassDefinition, oom};

#[derive(Clone)]
pub struct KtfJvmImplementation {
    core: ArmCore,
    java_functions: JavaSvcFunctions,
}

impl KtfJvmImplementation {
    pub fn new(core: &mut ArmCore) -> Self {
        let java_functions = Arc::new(Mutex::new(BTreeMap::new()));
        register_java_svc_handler(core, &java_functions).unwrap();

        Self {
            core: core.clone(),
            java_functions,
        }
    }

    pub fn java_functions(&self) -> JavaSvcFunctions {
        self.java_functions.clone()
    }
}

impl JvmImplementation for KtfJvmImplementation {
    fn define_class_rust<'a, C, Context>(
        &'a self,
        jvm: &'a Jvm,
        proto: JavaClassProto<C>,
        context: Context,
    ) -> Pin<Box<dyn Future<Output = JvmResult<Box<dyn ClassDefinition>>> + Send + 'a>>
    where
        C: ?Sized + 'static + Send,
        Context: Sync + Send + DerefMut + Deref<Target = C> + Clone + 'static,
    {
        Box::pin(async move {
            // Defining a class is a dozen guest allocations, so it fails first when the heap is gone. This used to `unwrap` — a panic on exactly
            // the path the OOM work is trying to survive. Report it as a java exception, through `oom` so that building the report cannot fail.
            match JavaClassDefinition::new(&mut self.core.clone(), jvm, proto, context, self.java_functions.clone()).await {
                Ok(x) => Ok(Box::new(x) as Box<dyn ClassDefinition>),
                Err(e) => {
                    let allocation_failure = matches!(e, WieError::AllocationFailure);

                    Err(oom::java_error(jvm, &self.core, allocation_failure, move || format!("Failed to define class: {e}")).await)
                }
            }
        })
    }

    async fn define_class_java(&self, _jvm: &Jvm, _data: &[u8]) -> JvmResult<Box<dyn ClassDefinition>> {
        unreachable!()
    }

    async fn define_array_class(&self, jvm: &Jvm, element_type_name: &str) -> JvmResult<Box<dyn ClassDefinition>> {
        let class_name = format!("[{element_type_name}");

        match JavaArrayClassDefinition::new(&mut self.core.clone(), jvm, &class_name).await {
            Ok(x) => Ok(Box::new(x) as Box<_>),
            Err(e) => {
                let allocation_failure = matches!(e, WieError::AllocationFailure);

                Err(oom::java_error(jvm, &self.core, allocation_failure, move || format!("Failed to define array class: {e}")).await)
            }
        }
    }
}
