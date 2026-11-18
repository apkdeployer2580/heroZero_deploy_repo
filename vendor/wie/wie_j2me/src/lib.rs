#![no_std]
extern crate alloc;

mod emulator;

pub use emulator::{J2MEEmulator, J2meOptions};
// ★ heroZero fork: so an app can build a `J2meOptions` without depending on wie_jvm_support.
pub use wie_jvm_support::{ClassTransformer, WieJavaClassProto};
