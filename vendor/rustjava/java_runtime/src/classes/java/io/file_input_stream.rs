use alloc::{format, vec};

use java_class_proto::{JavaFieldProto, JavaMethodProto};
use jvm::{Array, ClassInstanceRef, Jvm, Result, runtime::JavaLangString};

use crate::{
    RuntimeClassProto, RuntimeContext,
    classes::java::io::{File, FileDescriptor},
};

// class java.io.FileInputStream
pub struct FileInputStream;

impl FileInputStream {
    pub fn as_proto() -> RuntimeClassProto {
        RuntimeClassProto {
            name: "java/io/FileInputStream",
            parent_class: Some("java/io/InputStream"),
            interfaces: vec![],
            methods: vec![
                JavaMethodProto::new("<init>", "(Ljava/io/File;)V", Self::init, Default::default()),
                JavaMethodProto::new(
                    "<init>",
                    "(Ljava/io/FileDescriptor;)V",
                    Self::init_with_file_descriptor,
                    Default::default(),
                ),
                JavaMethodProto::new("read", "()I", Self::read_byte, Default::default()),
                JavaMethodProto::new("read", "([BII)I", Self::read_array, Default::default()),
                JavaMethodProto::new("available", "()I", Self::available, Default::default()),
                JavaMethodProto::new("close", "()V", Self::close, Default::default()),
            ],
            fields: vec![JavaFieldProto::new("fd", "Ljava/io/FileDescriptor;", Default::default())],
            access_flags: Default::default(),
        }
    }

    async fn init(jvm: &Jvm, context: &mut RuntimeContext, this: ClassInstanceRef<Self>, file: ClassInstanceRef<File>) -> Result<()> {
        tracing::debug!("java.io.FileInputStream::<init>({this:?}, {file:?})");

        let path = jvm.invoke_virtual(&file, "getPath", "()Ljava/lang/String;", ()).await?;
        let path = JavaLangString::to_rust_string(jvm, &path).await?;

        let fd = context.open(&path, false).await;
        if fd.is_err() {
            return Err(jvm.exception("java/io/FileNotFoundException", "File not found").await);
        }

        let fd = FileDescriptor::from_fd(jvm, fd.unwrap()).await?;

        let _: () = jvm
            .invoke_special(&this, "java/io/FileInputStream", "<init>", "(Ljava/io/FileDescriptor;)V", (fd,))
            .await?;

        Ok(())
    }

    async fn init_with_file_descriptor(
        jvm: &Jvm,
        _: &mut RuntimeContext,
        mut this: ClassInstanceRef<Self>,
        fd: ClassInstanceRef<FileDescriptor>,
    ) -> Result<()> {
        tracing::debug!("java.io.FileInputStream::<init>({this:?}, {fd:?})");

        let _: () = jvm.invoke_special(&this, "java/io/InputStream", "<init>", "()V", ()).await?;

        jvm.put_field(&mut this, "fd", "Ljava/io/FileDescriptor;", fd).await?;

        Ok(())
    }

    async fn available(jvm: &Jvm, context: &mut RuntimeContext, this: ClassInstanceRef<Self>) -> Result<i32> {
        tracing::debug!("java.io.FileInputStream::available({this:?})");

        let fd = jvm.get_field(&this, "fd", "Ljava/io/FileDescriptor;").await?;
        let rust_file = FileDescriptor::file(jvm, context, fd).await?;

        // TODO get os buffer size
        let stat = rust_file.metadata().await.unwrap();
        let tell = rust_file.tell().await.unwrap();

        let available = stat.size - tell;

        Ok(available as _)
    }

    async fn read_array(
        jvm: &Jvm,
        context: &mut RuntimeContext,
        this: ClassInstanceRef<Self>,
        mut buf: ClassInstanceRef<Array<i8>>,
        offset: i32,
        length: i32,
    ) -> Result<i32> {
        tracing::debug!("java.io.FileInputStream::read({this:?}, {buf:?}, {offset:?}, {length:?})");

        let fd = jvm.get_field(&this, "fd", "Ljava/io/FileDescriptor;").await?;
        let mut rust_file = FileDescriptor::file(jvm, context, fd).await?;

        // ★ heroZero fork (2026-10-22): the bytes go straight into the array (`array_raw_buffer_mut`),
        // and only the ones read. `store_array` widens every element to a 24-byte `JavaValue` first:
        // `ZipFile.<init>` reads the whole MIDlet jar through here, and for 영웅서기 Zero's 27 MB jar
        // that was a ~700 MB transient at boot — which Android's allocator then KEPT for the whole
        // session (S9 meminfo: native heap 816 MB, 724 MB of it free; host probe: 706 MB peak at
        // ~130 ms). The bounds check is the one `store_array` made, before anything is read.
        let array_length = jvm.array_length(&buf).await?;
        if offset < 0 || length < 0 || offset as usize + length as usize > array_length {
            return Err(jvm
                .exception(
                    "java/lang/ArrayIndexOutOfBoundsException",
                    &format!("{} > {array_length}", offset as i64 + length as i64),
                )
                .await);
        }

        let mut rust_buf = vec![0; length as _];
        let read = rust_file.read(&mut rust_buf).await.unwrap();
        if read == 0 {
            return Ok(-1);
        }

        jvm.array_raw_buffer_mut(&mut buf).await?.write(offset as _, &rust_buf[..read])?;

        Ok(read as _)
    }

    async fn read_byte(jvm: &Jvm, context: &mut RuntimeContext, this: ClassInstanceRef<Self>) -> Result<i32> {
        tracing::debug!("java.io.FileInputStream::read({this:?})");

        let fd = jvm.get_field(&this, "fd", "Ljava/io/FileDescriptor;").await?;
        let mut rust_file = FileDescriptor::file(jvm, context, fd).await?;

        let mut buf = [0; 1];
        let read = rust_file.read(&mut buf).await.unwrap();
        if read == 0 {
            return Ok(-1);
        }

        Ok(buf[0] as i32)
    }

    async fn close(jvm: &Jvm, context: &mut RuntimeContext, this: ClassInstanceRef<Self>) -> Result<()> {
        tracing::debug!("java.io.FileInputStream::close({this:?})");

        let fd = jvm.get_field(&this, "fd", "Ljava/io/FileDescriptor;").await?;
        FileDescriptor::close(jvm, context, fd).await?;

        Ok(())
    }
}
