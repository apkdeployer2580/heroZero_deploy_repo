use alloc::vec;

use jvm::{Result, runtime::JavaLangString};

use test_utils::test_jvm_filesystem;

#[tokio::test]
async fn test_file_input_stream_read_buf() -> Result<()> {
    let filesystem = [("test.txt".into(), b"hello world".to_vec())];
    let jvm = test_jvm_filesystem(filesystem.into_iter().collect()).await?;

    let file = JavaLangString::from_rust_string(&jvm, "test.txt").await?;

    let java_file = jvm.new_class("java/io/File", "(Ljava/lang/String;)V", (file,)).await?;
    let fis = jvm.new_class("java/io/FileInputStream", "(Ljava/io/File;)V", (java_file,)).await?;

    let bytes = jvm.instantiate_array("B", 11).await?;
    let read: i32 = jvm.invoke_virtual(&fis, "read", "([B)I", (bytes.clone(),)).await?;

    let length = jvm.array_length(&bytes).await?;
    let mut buf = vec![0; length];
    jvm.array_raw_buffer(&bytes).await?.read(0, &mut buf)?;

    assert_eq!(read, 11);
    assert_eq!(buf, vec![104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100]);

    Ok(())
}

#[tokio::test]
async fn test_file_input_stream_read_int() -> Result<()> {
    let filesystem = [("test.txt".into(), b"hello world".to_vec())];
    let jvm = test_jvm_filesystem(filesystem.into_iter().collect()).await?;

    let file = JavaLangString::from_rust_string(&jvm, "test.txt").await?;

    let java_file = jvm.new_class("java/io/File", "(Ljava/lang/String;)V", (file,)).await?;
    let fis = jvm.new_class("java/io/FileInputStream", "(Ljava/io/File;)V", (java_file,)).await?;

    let read: i32 = jvm.invoke_virtual(&fis, "read", "()I", ()).await?;

    assert_eq!(read, 104);

    Ok(())
}

#[tokio::test]
async fn test_file_input_stream_available() -> Result<()> {
    let filesystem = [("test.txt".into(), b"hello world".to_vec())];
    let jvm = test_jvm_filesystem(filesystem.into_iter().collect()).await?;

    let file = JavaLangString::from_rust_string(&jvm, "test.txt").await?;

    let java_file = jvm.new_class("java/io/File", "(Ljava/lang/String;)V", (file,)).await?;
    let fis = jvm.new_class("java/io/FileInputStream", "(Ljava/io/File;)V", (java_file,)).await?;

    let available: i32 = jvm.invoke_virtual(&fis, "available", "()I", ()).await?;

    assert!(available > 0);

    Ok(())
}

#[tokio::test]
async fn test_file_input_stream_sequential_read() -> Result<()> {
    let filesystem = [("test.txt".into(), b"hello".to_vec())];
    let jvm = test_jvm_filesystem(filesystem.into_iter().collect()).await?;

    let file = JavaLangString::from_rust_string(&jvm, "test.txt").await?;

    let java_file = jvm.new_class("java/io/File", "(Ljava/lang/String;)V", (file,)).await?;
    let fis = jvm.new_class("java/io/FileInputStream", "(Ljava/io/File;)V", (java_file,)).await?;

    let mut reads = vec![];
    for _ in 0..6 {
        reads.push(jvm.invoke_virtual::<_, i32>(&fis, "read", "()I", ()).await?);
    }

    assert_eq!(reads, vec![104, 101, 108, 108, 111, -1]);

    Ok(())
}

#[tokio::test]
async fn test_file_input_stream_skip_past_eof() -> Result<()> {
    let filesystem = [("test.txt".into(), b"hello".to_vec())];
    let jvm = test_jvm_filesystem(filesystem.into_iter().collect()).await?;

    let file = JavaLangString::from_rust_string(&jvm, "test.txt").await?;

    let java_file = jvm.new_class("java/io/File", "(Ljava/lang/String;)V", (file,)).await?;
    let fis = jvm.new_class("java/io/FileInputStream", "(Ljava/io/File;)V", (java_file,)).await?;

    let skipped: i64 = jvm.invoke_virtual(&fis, "skip", "(J)J", (10i64,)).await?;
    assert_eq!(skipped, 5);

    let read: i32 = jvm.invoke_virtual(&fis, "read", "()I", ()).await?;
    assert_eq!(read, -1);

    Ok(())
}

// ★ heroZero fork (2026-10-22): `read([BII)` writes the bytes in place — at the offset, only the
// ones read, the rest of the array untouched — and refuses a range past the array's end.
#[tokio::test]
async fn test_file_input_stream_read_into_the_middle_of_an_array() -> Result<()> {
    let filesystem = [("test.txt".into(), b"hey".to_vec())];
    let jvm = test_jvm_filesystem(filesystem.into_iter().collect()).await?;

    let file = JavaLangString::from_rust_string(&jvm, "test.txt").await?;

    let java_file = jvm.new_class("java/io/File", "(Ljava/lang/String;)V", (file,)).await?;
    let fis = jvm.new_class("java/io/FileInputStream", "(Ljava/io/File;)V", (java_file,)).await?;

    let mut bytes = jvm.instantiate_array("B", 8).await?;
    jvm.array_raw_buffer_mut(&mut bytes).await?.write(0, &[9; 8])?;
    let read: i32 = jvm.invoke_virtual(&fis, "read", "([BII)I", (bytes.clone(), 2, 5)).await?;

    let mut buf = vec![0; 8];
    jvm.array_raw_buffer(&bytes).await?.read(0, &mut buf)?;

    assert_eq!(read, 3);
    assert_eq!(buf, vec![9, 9, 104, 101, 121, 9, 9, 9]);

    let past_the_end: Result<i32> = jvm.invoke_virtual(&fis, "read", "([BII)I", (bytes.clone(), 6, 5)).await;
    assert!(past_the_end.is_err());

    Ok(())
}
