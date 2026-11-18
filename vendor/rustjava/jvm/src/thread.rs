use alloc::{boxed::Box, sync::Arc, vec::Vec};

use crate::{ClassInstance, class_loader::Class};

pub enum StackFrame {
    Java(JavaStackFrame),
    Native(NativeStackFrame),
}

impl StackFrame {
    pub fn local_variables(&self) -> &[Box<dyn ClassInstance>] {
        match self {
            StackFrame::Java(java_frame) => &java_frame.local_variables,
            StackFrame::Native(native_frame) => &native_frame.local_variables,
        }
    }

    pub fn local_variables_mut(&mut self) -> &mut Vec<Box<dyn ClassInstance>> {
        match self {
            StackFrame::Java(java_frame) => &mut java_frame.local_variables,
            StackFrame::Native(native_frame) => &mut native_frame.local_variables,
        }
    }

    /// Everything in this frame that keeps an object alive.
    ///
    /// ★ This is `local_variables` PLUS the receiver. `determine_garbage` used to scan only the
    /// locals, so `this` was not a root for the duration of its own method - a hole that any
    /// JVM has to close, and one that costs a freed-out-from-under-you object on the KTF backend,
    /// where every instance is guest memory the allocator will hand out again.
    pub fn roots(&self) -> impl Iterator<Item = &Box<dyn ClassInstance>> {
        let this = match self {
            StackFrame::Java(java_frame) => java_frame.class_instance.as_ref(),
            StackFrame::Native(_) => None,
        };

        self.local_variables().iter().chain(this)
    }
}

pub struct JvmThread {
    stack: Vec<StackFrame>,
}

impl JvmThread {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn push_java_frame(&mut self, class: &Class, class_instance: Option<Box<dyn ClassInstance>>, method: Arc<str>) {
        self.stack.push(StackFrame::Java(JavaStackFrame {
            class: class.clone(),
            class_instance,
            method,
            local_variables: Vec::new(),
        }));
    }

    pub fn push_native_frame(&mut self) {
        self.stack.push(StackFrame::Native(NativeStackFrame { local_variables: Vec::new() }));
    }

    pub fn pop_frame(&mut self) -> Option<StackFrame> {
        self.stack.pop()
    }

    pub fn top_frame_mut(&mut self) -> &mut StackFrame {
        self.stack.last_mut().unwrap()
    }

    pub fn top_java_frame(&self) -> Option<&JavaStackFrame> {
        self.stack.iter().rev().find_map(|frame| match frame {
            StackFrame::Java(java_frame) => Some(java_frame),
            _ => None,
        })
    }

    pub fn iter_java_frame(&self) -> impl DoubleEndedIterator<Item = &JavaStackFrame> {
        self.stack.iter().filter_map(|frame| match frame {
            StackFrame::Java(java_frame) => Some(java_frame),
            _ => None,
        })
    }

    pub fn iter_frame(&self) -> impl DoubleEndedIterator<Item = &StackFrame> {
        self.stack.iter()
    }
}

pub struct JavaStackFrame {
    pub class: Class,
    pub class_instance: Option<Box<dyn ClassInstance>>,
    /// `name + descriptor` — shared with the method, not copied per call.
    pub method: Arc<str>,
    pub local_variables: Vec<Box<dyn ClassInstance>>,
}

pub struct NativeStackFrame {
    pub local_variables: Vec<Box<dyn ClassInstance>>,
}
