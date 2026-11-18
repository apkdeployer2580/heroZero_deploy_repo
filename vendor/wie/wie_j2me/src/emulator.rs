use alloc::{
    borrow::ToOwned,
    boxed::Box,
    collections::btree_map::BTreeMap,
    format, str,
    string::{String, ToString},
    vec::Vec,
};

use jvm::{
    ClassInstance, Result as JvmResult,
    runtime::{JavaIoInputStream, JavaLangString},
};

use wie_backend::{DefaultTaskRunner, Emulator, Event, Platform, System};
use wie_jvm_support::{ClassTransformer, JvmSupport, RustJavaJvmImplementation, TransformingJvmImplementation, WieJavaClassProto};
use wie_util::{Result, WieError};

pub struct J2MEEmulator {
    system: System,
}

/// ★ heroZero fork (2026-10-21): what an app may change about how a MIDlet is started,
/// without touching the jar. Both are how 영웅서기 Zero's own patches are applied (the app's
/// `hz_patches` crate owns the policy; this is only the mechanism).
#[derive(Default)]
pub struct J2meOptions {
    /// App properties layered OVER the descriptor's — on a clash these win. What
    /// `MIDlet.getAppProperty` returns.
    pub property_overrides: Vec<(String, String)>,
    /// Rewrites class files on their way into the JVM. See [`ClassTransformer`].
    pub class_transformer: Option<ClassTransformer>,
    /// ★ heroZero fork (2026-10-22): host classes defined next to `wie_midp`'s, found by name
    /// like any other. Lets an app give a rewritten class something of its own to call (the
    /// hooks a `class_transformer` patch invokes) without that class living in the platform.
    pub extra_protos: Vec<WieJavaClassProto>,
}

impl J2MEEmulator {
    pub fn from_jad_jar(platform: Box<dyn Platform>, jad: Vec<u8>, jar_filename: String, jar: Vec<u8>) -> Result<Self> {
        let descriptor = J2MEDescriptor::parse(&jad);

        let files = [(jar_filename.to_owned(), jar)].into_iter().collect();
        Self::load(
            platform,
            &jar_filename,
            &descriptor.name,
            Some(descriptor.main_class_name),
            descriptor.properties,
            &files,
            J2meOptions::default(),
        )
    }

    pub fn from_jar(platform: Box<dyn Platform>, jar_filename: &str, jar: Vec<u8>) -> Result<Self> {
        Self::from_jar_with(platform, jar_filename, jar, J2meOptions::default())
    }

    /// ★ heroZero fork: [`Self::from_jar`] with [`J2meOptions`]. The app id (which keys the
    /// RecordStores, i.e. the saves) is `jar_filename`, exactly as `from_jar`.
    pub fn from_jar_with(platform: Box<dyn Platform>, jar_filename: &str, jar: Vec<u8>, options: J2meOptions) -> Result<Self> {
        let files = [(jar_filename.to_owned(), jar)].into_iter().collect();

        Self::load(platform, jar_filename, jar_filename, None, BTreeMap::new(), &files, options)
    }

    fn load(
        platform: Box<dyn Platform>,
        jar_filename: &str,
        id: &str,
        main_class_name: Option<String>,
        properties: BTreeMap<String, String>,
        files: &BTreeMap<String, Vec<u8>>,
        options: J2meOptions,
    ) -> Result<Self> {
        let system = System::new(platform, id, id, DefaultTaskRunner);

        for (path, data) in files {
            system.filesystem().add_virtual(path, data.clone());
        }

        let mut system_clone = system.clone();
        let jar_filename = jar_filename.to_owned();

        system.spawn(async move || Self::do_start(&mut system_clone, jar_filename, properties, main_class_name, options).await);

        Ok(J2MEEmulator { system })
    }

    #[tracing::instrument(name = "start", skip_all)]
    async fn do_start(
        system: &mut System,
        jar_filename: String,
        properties: BTreeMap<String, String>,
        main_class_name: Option<String>,
        options: J2meOptions,
    ) -> Result<()> {
        let mut properties = properties
            .into_iter()
            .map(|(k, v)| (format!("wie.appProperty.{k}"), v))
            .collect::<Vec<_>>();
        // ★ heroZero fork (2026-10-22): a plain MIDlet expects Nokia-style key codes (soft keys
        // −6/−7, arrows −1..−4), not the SKVM codes the shared event queue carries. See
        // `wie_midp::classes::net::wie::KeyCodeProfile`.
        properties.push((
            wie_midp::classes::net::wie::KeyCodeProfile::PROPERTY.to_string(),
            "nokia".to_string(),
        ));
        let properties = properties.iter().map(|(k, v)| (k.as_ref(), v.as_ref())).collect::<Vec<_>>();

        let protos = [wie_midp::get_protos().into(), options.extra_protos.into_boxed_slice()];
        let jvm = match options.class_transformer {
            Some(t) => {
                let implementation = TransformingJvmImplementation::new(t);
                JvmSupport::new_jvm(system, Some(&jar_filename), Box::new(protos), &properties, implementation).await?
            }
            None => JvmSupport::new_jvm(system, Some(&jar_filename), Box::new(protos), &properties, RustJavaJvmImplementation).await?,
        };

        let main_class_name = if let Some(x) = main_class_name {
            x.replace('.', "/")
        } else {
            let class_loader = jvm
                .invoke_static("java/lang/ClassLoader", "getSystemClassLoader", "()Ljava/lang/ClassLoader;", ())
                .await
                .unwrap();

            // TODO Use JarFile::getManifest
            let resource_name = JavaLangString::from_rust_string(&jvm, "META-INF/MANIFEST.MF").await.unwrap();
            let resource_stream = jvm
                .invoke_virtual(
                    &class_loader,
                    "getResourceAsStream",
                    "(Ljava/lang/String;)Ljava/io/InputStream;",
                    (resource_name.clone(),),
                )
                .await
                .unwrap();
            let data = JavaIoInputStream::read_until_end(&jvm, &resource_stream).await.unwrap();

            let descriptor = J2MEDescriptor::parse(&data);

            for (k, v) in descriptor.properties {
                let property_key = format!("wie.appProperty.{k}");
                let property_key = JavaLangString::from_rust_string(&jvm, &property_key).await.unwrap();
                let property_value = JavaLangString::from_rust_string(&jvm, &v).await.unwrap();

                let _: Option<Box<dyn ClassInstance>> = jvm
                    .invoke_static(
                        "java/lang/System",
                        "setProperty",
                        "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Object;",
                        (property_key, property_value),
                    )
                    .await
                    .unwrap();
            }

            if descriptor.main_class_name.is_empty() {
                return Err(WieError::FatalError("Main class not found".into()));
            }
            descriptor.main_class_name.replace('.', "/")
        };

        // ★ heroZero fork: the overrides go in LAST, after the descriptor's properties, so
        // they win on a clash whichever constructor was used.
        for (k, v) in &options.property_overrides {
            tracing::info!("app property override {k} = {v}");
            let property_key = JavaLangString::from_rust_string(&jvm, &format!("wie.appProperty.{k}")).await.unwrap();
            let property_value = JavaLangString::from_rust_string(&jvm, v).await.unwrap();
            let _: Option<Box<dyn ClassInstance>> = jvm
                .invoke_static(
                    "java/lang/System",
                    "setProperty",
                    "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Object;",
                    (property_key, property_value),
                )
                .await
                .unwrap();
        }

        let main_class_java = JavaLangString::from_rust_string(&jvm, &main_class_name).await.unwrap();

        let result: JvmResult<()> = jvm
            .invoke_static("net/wie/Launcher", "start", "(Ljava/lang/String;)V", (main_class_java,))
            .await;

        if let Err(x) = result {
            return Err(JvmSupport::to_wie_err(&jvm, x).await);
        }

        Ok(())
    }
}

impl Emulator for J2MEEmulator {
    fn handle_event(&mut self, event: Event) {
        // ★ heroZero fork (2026-10-28): wakes the event pump if it is napping (`System::push_event`).
        self.system.push_event(event)
    }

    fn tick(&mut self) -> Result<()> {
        self.system.tick()
    }

    // ★ heroZero fork (2026-10-21): as `wie_ktf` does — without it the host worker loop has
    // no idea when the guest's next timer is due and falls back to a fixed pace.
    fn next_wakeup_ms(&self) -> Option<u64> {
        self.system.next_wakeup_ms()
    }
}

struct J2MEDescriptor {
    name: String,
    main_class_name: String,
    properties: BTreeMap<String, String>,
}

impl J2MEDescriptor {
    pub fn parse(data: &[u8]) -> Self {
        let lines = data.split(|x| *x == b'\n');

        let mut name = String::new();
        let mut main_class_name = String::new();
        let mut properties = BTreeMap::new();

        for line in lines {
            let line = str::from_utf8(line).unwrap().trim();

            if line.is_empty() {
                continue;
            }

            let mut parts = line.splitn(2, ':');

            let key = parts.next().unwrap().trim();
            let value = parts.next().unwrap().trim();
            properties.insert(key.trim().to_string(), value.trim().to_string());

            match key {
                "MIDlet-Name" => name = value.to_string(),
                "MIDlet-1" => main_class_name = value.split(',').nth(2).unwrap().trim().to_string(),
                _ => {}
            }
        }

        Self {
            name,
            main_class_name,
            properties,
        }
    }
}
