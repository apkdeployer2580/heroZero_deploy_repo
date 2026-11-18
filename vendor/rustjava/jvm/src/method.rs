use alloc::{boxed::Box, format, string::String, sync::Arc};
use core::fmt::Debug;

use java_constants::MethodAccessFlags;

use crate::{JavaValue, Jvm, Result, as_any::AsAny};

#[async_trait::async_trait]
pub trait Method: Sync + Send + AsAny + Debug {
    fn name(&self) -> String;
    fn descriptor(&self) -> String;
    fn access_flags(&self) -> MethodAccessFlags;

    /// `name + descriptor`, as a stack frame records it. The default builds it from the two
    /// accessors; a backend that already holds the string memoised (wie's KTF name records)
    /// overrides this so a method call does not allocate three strings for its frame.
    fn signature(&self) -> Arc<str> {
        format!("{}{}", self.name(), self.descriptor()).into()
    }

    async fn run(&self, jvm: &Jvm, args: Box<[JavaValue]>) -> Result<JavaValue>;
}
