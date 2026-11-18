use jvm::Result;

use test_utils::test_jvm;

#[tokio::test]
async fn test_mark_reset() -> Result<()> {
    let jvm = test_jvm().await?;

    let mut buffer = jvm.instantiate_array("B", 5).await?;
    jvm.array_raw_buffer_mut(&mut buffer).await?.write(0, &[10, 20, 30, 40, 50])?;

    let stream = jvm.new_class("java/io/ByteArrayInputStream", "([B)V", (buffer,)).await?;

    let first: i32 = jvm.invoke_virtual(&stream, "read", "()I", ()).await?;
    assert_eq!(first, 10);

    let _: () = jvm.invoke_virtual(&stream, "mark", "(I)V", (100,)).await?;

    let second: i32 = jvm.invoke_virtual(&stream, "read", "()I", ()).await?;
    assert_eq!(second, 20);

    let _: () = jvm.invoke_virtual(&stream, "reset", "()V", ()).await?;

    let again: i32 = jvm.invoke_virtual(&stream, "read", "()I", ()).await?;
    assert_eq!(again, 20);

    Ok(())
}

// ★ heroZero fork (2026-11-03): a null array is Java's NullPointerException, never a host panic.
#[tokio::test]
async fn test_null_array_throws_null_pointer() -> Result<()> {
    let jvm = test_jvm().await?;

    for (descriptor, args) in [
        ("([B)V", vec![jvm::JavaValue::Object(None)]),
        ("([BII)V", vec![jvm::JavaValue::Object(None), jvm::JavaValue::Int(0), jvm::JavaValue::Int(4)]),
    ] {
        let made = jvm.new_class("java/io/ByteArrayInputStream", descriptor, args).await;
        let Err(jvm::JavaError::JavaException(e)) = made else {
            panic!("{descriptor}: not a Java exception");
        };
        assert!(jvm.is_instance(&*e, "java/lang/NullPointerException"), "{descriptor}");
    }

    Ok(())
}
