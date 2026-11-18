use std::{collections::HashMap, sync::Arc};

use crate::error::LoadError;

use super::generator::{GeneratorList, GeneratorType};
use super::modulator::Mod;
use super::{instrument::Instrument, Sample};

const GEN_SET: u32 = 1;

/// Imported instruments, keyed by their index in the SoundFont's instrument table, so
/// each one is built once per font instead of once per referencing preset zone.
/// See the note on [`PresetZone::inst`]. ★ wie fork 2026-08-09.
pub(crate) type InstrumentCache = HashMap<u16, Arc<Instrument>>;

pub struct Preset {
    name: String,
    bank: u32,
    num: u32,
    global_zone: Option<PresetZone>,
    zones: Vec<PresetZone>,
}

impl Preset {
    pub(crate) fn import(
        sf2: &soundfont::SoundFont2,
        preset: &soundfont::Preset,
        samples: &[Sample],
        cache: &mut InstrumentCache,
    ) -> Result<Self, LoadError> {
        let name = if !preset.header.name.is_empty() {
            preset.header.name.clone()
        } else {
            format!("Bank:{},Preset{}", preset.header.bank, preset.header.preset)
        };

        let mut global_zone = None;
        let mut zones = Vec::new();

        for (id, sfzone) in preset.zones.iter().enumerate() {
            let name = format!("{}/{}", preset.header.name, id);
            let zone = PresetZone::import(name, sf2, sfzone, samples, cache)?;

            if id == 0 && zone.inst.is_none() {
                global_zone = Some(zone);
            } else {
                zones.push(zone);
            }
        }

        Ok(Self {
            name,
            bank: preset.header.bank as u32,
            num: preset.header.preset as u32,
            global_zone,
            zones,
        })
    }

    pub(crate) fn global_zone(&self) -> Option<&PresetZone> {
        self.global_zone.as_ref()
    }

    pub(crate) fn zones(&self) -> &[PresetZone] {
        &self.zones
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn banknum(&self) -> u32 {
        self.bank
    }

    pub fn num(&self) -> u32 {
        self.num
    }
}

pub struct PresetZone {
    #[allow(dead_code)]
    pub name: String,
    /// ★ wie fork 2026-08-09: `Arc`, was `Option<Instrument>` by value.
    ///
    /// `PresetZone::import` deep-copied a whole `Instrument` for EVERY preset zone that
    /// referenced one. GeneralUser.sf3 has 324 instruments and 2,900 preset zones, so
    /// the loader built 24,155 `InstrumentZone`s where the file describes 2,482 — 9.7x —
    /// and an `InstrumentZone` is 2,048 bytes (its `GeneratorList` alone is
    /// `[Generator; 60]` = 1,920). That is ~47 MiB of pure duplication, on top of the
    /// ~30 MiB of decoded PCM which was already shared correctly.
    ///
    /// Measured on the target device (Galaxy S9, 3.6 GB, Android 10): native heap 271 MB
    /// with the phone already 921 MB into swap. 29차 established that the only surviving
    /// cause of this app being killed is the low-memory killer, so this is not cosmetic.
    ///
    /// Safe as a shared reference: `inst` is READ-ONLY after load — the only two uses in
    /// the whole crate are `core/midi.rs`'s `preset_zone.inst.as_ref()` when a voice
    /// starts, and the `zone.inst.is_none()` global-zone test in `Preset::import`.
    pub inst: Option<Arc<Instrument>>,
    pub key_low: u8,
    pub key_high: u8,
    pub vel_low: u8,
    pub vel_high: u8,
    pub gen: GeneratorList,
    pub mods: Vec<Mod>,
}

impl PresetZone {
    pub fn import(
        name: String,
        sf2: &soundfont::SoundFont2,
        zone: &soundfont::Zone,
        samples: &[Sample],
        cache: &mut InstrumentCache,
    ) -> Result<Self, LoadError> {
        let mut key_low = 0;
        let mut key_high = 128;
        let mut vel_low = 0;
        let mut vel_high = 128;

        let mut gen = GeneratorList::default();

        for sfgen in zone
            .gen_list
            .iter()
            .filter(|g| g.ty != soundfont::raw::GeneratorType::Instrument)
        {
            let Ok(ty) = sfgen.ty.into_result() else {
                continue;
            };

            match ty {
                soundfont::raw::GeneratorType::KeyRange => {
                    let amount = sfgen.amount.as_range().unwrap();
                    key_low = amount.low;
                    key_high = amount.high;
                }
                soundfont::raw::GeneratorType::VelRange => {
                    let amount = sfgen.amount.as_range().unwrap();
                    vel_low = amount.low;
                    vel_high = amount.high;
                }
                _ => {
                    if let Ok(ty) = GeneratorType::try_from(ty) {
                        // FIXME: some generators have an unsigned word amount value but i don't know which ones
                        gen[ty].val = *sfgen.amount.as_i16().unwrap() as f64;
                        gen[ty].flags = GEN_SET as u8;
                    }
                }
            }
        }

        let inst = if let Some(id) = zone.instrument() {
            // One import per DISTINCT instrument id, shared by every zone that names it.
            Some(match cache.get(id) {
                Some(existing) => existing.clone(),
                None => {
                    let made = Arc::new(Instrument::import(sf2, &sf2.instruments[*id as usize], samples)?);
                    cache.insert(*id, made.clone());
                    made
                }
            })
        } else {
            None
        };

        // Import the modulators (only SF2.1 and higher)
        let mods: Vec<_> = zone
            .mod_list
            .iter()
            .map(|mod_src| {
                // Store the new modulator in the zone The order of modulators
                // will make a difference, at least in an instrument context: The
                // second modulator overwrites the first one, if they only differ
                // in amount.
                Mod::from(mod_src)
            })
            .collect();

        Ok(Self {
            name,
            inst,
            key_low,
            key_high,
            vel_low,
            vel_high,
            gen,
            mods,
        })
    }
}
