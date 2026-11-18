use alloc::{boxed::Box, collections::BTreeMap, rc::Rc, string::String, vec::Vec};
use java_constants::FieldAccessFlags;

use hashbrown::{HashMap, HashSet, hash_set::Entry};

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::{ClassDefinition, ClassInstance, Field, JavaValue, Jvm, class_loader::Class, thread::JvmThread};

/// True while a collection is in progress.
///
/// ★ It lives HERE rather than at a call site because there are three call sites and the guest
/// picks which one runs: `Display::handlePaintEvent` (throttled by wie's `--gc` period) and
/// `System.gc()` / `Runtime.gc()`, which the guest calls itself and nothing throttles. hero2 does
/// call `System.gc()`, so a flag set only around the paint-path call would have been false for
/// most real collections - and a backend that reads object fields out of emulated memory needs to
/// know whether a failed read was the collector following a dangling edge or the guest using one.
static COLLECTING: AtomicBool = AtomicBool::new(false);

pub fn is_collecting() -> bool {
    COLLECTING.load(Ordering::Relaxed)
}

/// Clears the flag on every exit path, including a panic unwinding through the collector.
pub struct CollectingGuard;

impl CollectingGuard {
    pub fn enter() -> Self {
        COLLECTING.store(true, Ordering::Relaxed);

        Self
    }
}

impl Drop for CollectingGuard {
    fn drop(&mut self) {
        COLLECTING.store(false, Ordering::Relaxed);
    }
}

/// ★★ WHAT THE MARK WALK NEEDS TO KNOW ABOUT A CLASS — worked out ONCE PER COLLECTION
/// instead of once per object (2026-09-18).
///
/// The walk used to call [`find_all_fields`] for every reachable object, and that rebuilds
/// the whole inherited field list from scratch: a `Vec<Box<dyn Field>>` per class in the
/// chain, a `String` for every superclass name, and then another `String` for every field
/// descriptor just to test its first byte. On the KTF backend each of those is also a READ
/// OUT OF EMULATED GUEST MEMORY. With ~4,000 live objects that is tens of thousands of
/// allocations and guest reads per collection — and on wie this collection runs every
/// painted frame.
///
/// Measured with the host phase profiler (roadmap #9) before this cache: **gc was 50.5 % of
/// the emulator thread's busy time**, 8.7 ms of host time per painted frame, with the ARM
/// interpreter itself at 10 %.
///
/// The cache lives for ONE collection and dies with it, so nothing in it can go stale: a
/// class registered or unloaded between collections is simply looked up again.
struct ClassGcInfo {
    /// `[L...` or `[[...` — the elements are references, so the walk recurses into them.
    object_array: bool,
    /// Non-static fields whose descriptor says "reference". A primitive field can never
    /// point at anything, so the walk has no reason to read it at all.
    ref_fields: Vec<Box<dyn Field>>,
}

/// Class name -> its [`ClassGcInfo`], built on first sight during this collection.
type GcClassCache = HashMap<String, Rc<ClassGcInfo>>;

fn is_reference(descriptor: &str) -> bool {
    (descriptor.starts_with('L') && descriptor.ends_with(';')) || descriptor.starts_with('[')
}

fn class_gc_info(jvm: &Jvm, class_definition: &dyn ClassDefinition, name: &str, cache: &mut GcClassCache) -> Rc<ClassGcInfo> {
    if let Some(hit) = cache.get(name) {
        return hit.clone();
    }

    let object_array = name.starts_with("[L") || name.starts_with("[[");
    let ref_fields = if name.starts_with('[') {
        Vec::new()
    } else {
        find_all_fields(jvm, class_definition)
            .into_iter()
            .filter(|f| !f.access_flags().contains(FieldAccessFlags::STATIC) && is_reference(&f.descriptor()))
            .collect()
    };

    let built = Rc::new(ClassGcInfo { object_array, ref_fields });
    cache.insert(String::from(name), built.clone());

    built
}

pub fn determine_garbage(
    jvm: &Jvm,
    threads: &BTreeMap<u64, JvmThread>,
    all_class_instances: &HashSet<Box<dyn ClassInstance>>,
    classes: &BTreeMap<String, Class>,
    interned_strings: &[Box<dyn ClassInstance>],
    pinned: &[Box<dyn ClassInstance>],
    conservative: &HashSet<u64>,
) -> Vec<Box<dyn ClassInstance>> {
    // ★ Identities, not boxed clones (2026-09-19). The set used to hold a
    // `Box<dyn ClassInstance>` clone of every reachable object - one heap allocation per
    // object per collection, ~8,000 a frame on hero2 - purely to remember that it had been
    // seen. See `ClassInstance::gc_identity` for why a hash-derived identity can only err
    // towards keeping an object.
    let mut reachable_objects: HashSet<u64> = HashSet::with_capacity(all_class_instances.len());
    let mut cache: GcClassCache = HashMap::new();

    classes.values().for_each(|x| {
        x.with_java_class(|mirror| find_reachable_objects(jvm, mirror, &mut reachable_objects, &mut cache));
        find_static_reachable_objects(jvm, x, &mut reachable_objects, &mut cache);
    });

    threads
        .values()
        .flat_map(|thread| thread.iter_frame().flat_map(|stack| stack.roots()))
        .for_each(|x| {
            find_reachable_objects(jvm, &**x, &mut reachable_objects, &mut cache);
        });

    interned_strings.iter().for_each(|x| {
        find_reachable_objects(jvm, &**x, &mut reachable_objects, &mut cache);
    });

    // Objects host code is holding across an await. See `Jvm::pin_object`.
    pinned.iter().for_each(|x| {
        find_reachable_objects(jvm, &**x, &mut reachable_objects, &mut cache);
    });

    // ★★ Objects the BACKEND'S OWN MACHINE STATE is holding — a register or an emulated
    // stack slot of AOT-compiled guest code, which nothing above can see. See
    // `Jvm::set_conservative_root_scanner` for why a JVM needs this at all.
    //
    // ⛔ These are marked as ROOTS, not merely spared from the sweep. An object reached
    // only from a register still owns its own field graph, and sparing it while freeing its
    // children would leave it holding dangling references — the same crash one collection
    // later, and far harder to read.
    //
    // One pass over the object table with an O(1) probe each. The pass is not extra work
    // the collector would otherwise avoid: the sweep filter below walks the same table.
    //
    // ★ It runs LAST on purpose: everything above has already marked what it can reach, so
    // an object still unmarked here is one that ONLY the guest's machine state is holding —
    // which is exactly the number worth counting.
    if !conservative.is_empty() {
        // Once per process: the scan is installed and produced words. Without this the only
        // evidence a backend is wired up at all is a crash that no longer happens.
        if !SCAN_ANNOUNCED.swap(true, Ordering::Relaxed) {
            tracing::warn!("gc: conservative root scan active — {} guest word(s) offered", conservative.len());
        }
        let mut rescued = 0usize;
        for object in all_class_instances.iter() {
            let Some(address) = object.gc_raw_address() else { continue };
            if !conservative.contains(&address) || reachable_objects.contains(&object.gc_identity()) {
                continue;
            }
            rescued += 1;
            find_reachable_objects(jvm, &**object, &mut reachable_objects, &mut cache);
        }
        report_conservative_rescue(rescued, conservative.len());
    }

    all_class_instances
        .iter()
        .filter(|x| !reachable_objects.contains(&x.gc_identity()))
        .cloned()
        .collect()
}

/// How many collections the conservative scan has saved an object in, so the report can
/// back off. Backing off matters: on a backend where this fires every frame an unthrottled
/// line is ~20 log writes a second FROM THE EMULATOR THREAD, which is the shape of mistake
/// the wie fork already had to undo once.
static RESCUES: AtomicUsize = AtomicUsize::new(0);

/// Set the first time a backend's scanner yields anything. See its one use.
static SCAN_ANNOUNCED: AtomicBool = AtomicBool::new(false);

/// Say — at 1st, 2nd, 4th, 8th … — that the conservative scan kept something nothing else
/// could see. That is not a warning about this collection; it is the ONLY evidence that the
/// scan is wired up and doing its job, and the crash it exists to stop leaves no trace when
/// it does not happen.
fn report_conservative_rescue(rescued: usize, scanned: usize) {
    if rescued == 0 {
        return;
    }
    let n = RESCUES.fetch_add(1, Ordering::Relaxed) + 1;
    if n.is_power_of_two() {
        tracing::warn!("gc: conservative scan kept {rescued} object(s) from {scanned} guest word(s) (occurrence {n})");
    }
}

fn find_static_reachable_objects(jvm: &Jvm, class: &Class, reachable_objects: &mut HashSet<u64>, cache: &mut GcClassCache) {
    // The backend's own walk, if it has one. Every registered class gets this call, so a
    // backend only needs to cover the class's OWN statics - which is also why the generic
    // path below, which walks the hierarchy, marks a parent's statics more than once.
    if class
        .definition
        .gc_visit_static_references(&mut |child| find_reachable_objects(jvm, child, reachable_objects, cache))
    {
        return;
    }

    let fields = find_all_fields(jvm, &*class.definition);
    for field in fields {
        if !field.access_flags().contains(FieldAccessFlags::STATIC) {
            continue;
        }

        let descriptor = field.descriptor();

        if is_reference(&descriptor) {
            // A read can fail on a backend where fields live in emulated memory. Skipping the
            // static is the conservative direction only because the object it points at stays in
            // `all_objects` and will be swept - so the log line matters; see the note on
            // `find_reachable_objects`.
            let Ok(value) = class.definition.get_static_field(&*field) else {
                tracing::error!("Could not read static {}.{} while marking; skipping it", class.definition.name(), field.name());
                continue;
            };
            if let JavaValue::Object(Some(value)) = value {
                find_reachable_objects(jvm, &*value, reachable_objects, cache);
            }
        }
    }
}

fn find_reachable_objects(jvm: &Jvm, object: &dyn ClassInstance, reachable_objects: &mut HashSet<u64>, cache: &mut GcClassCache) {
    let entry = reachable_objects.entry(object.gc_identity());
    if let Entry::Occupied(_) = entry {
        return;
    }
    entry.insert();

    // The backend's own walk, if it has one - no class lookup, no field objects, no boxed
    // values. The generic walk below is what it replaces.
    if object.gc_visit_references(&mut |child| find_reachable_objects(jvm, child, reachable_objects, cache)) {
        return;
    }

    let definition = object.class_definition();
    let name = definition.name();
    let info = class_gc_info(jvm, &*definition, &name, cache);

    if name.starts_with('[') {
        if info.object_array {
            // is object array
            let Some(array) = object.as_array_instance() else { return };
            let Ok(values) = array.load(0, array.length()) else { return };

            for value in values {
                if let JavaValue::Object(Some(value)) = value {
                    find_reachable_objects(jvm, &*value, reachable_objects, cache);
                }
            }
        }
        // do nothing for primitive arrays
    } else {
        for field in &info.ref_fields {
            // ⛔ This was `.unwrap()`, and on the KTF backend it is the statement that killed
            // the emulator worker: the mark walk followed a field still pointing at an object a
            // PREVIOUS collection had freed, and reading that corpse's own fields failed. The
            // object stays in `reachable_objects` (it was inserted at the top of this
            // function), so refusing to recurse is also refusing to free it a second time.
            let Ok(value) = object.get_field(&**field) else { continue };
            if let JavaValue::Object(Some(value)) = value {
                find_reachable_objects(jvm, &*value, reachable_objects, cache);
            }
        }
    }
}

fn find_all_fields(jvm: &Jvm, class_definition: &dyn ClassDefinition) -> Vec<Box<dyn Field>> {
    let result = class_definition.fields();
    let super_class_name = class_definition.super_class_name();

    if let Some(x) = super_class_name {
        // An unresolved superclass means its fields are invisible to the walk, which under-marks
        // rather than over-marks - so say it out loud instead of taking the whole collector down.
        let Some(super_class) = jvm.get_class(&x) else {
            tracing::error!("Superclass {x} of {} is not loaded; its fields will not be marked", class_definition.name());
            return result;
        };
        let super_fields = find_all_fields(jvm, &*super_class.definition);
        result.into_iter().chain(super_fields).collect()
    } else {
        result
    }
}
