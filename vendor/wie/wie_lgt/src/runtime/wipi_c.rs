use alloc::{boxed::Box, string::ToString, vec};

mod context;

use jvm::{Jvm, Result as JvmResult, runtime::JavaLangString};
use jvm_rust::ClassDefinitionImpl;
use wipi_types::lgt::CletFunctions;
use wipi_types::wipic::WIPICIndirectPtr;

use wie_backend::System;
use wie_core_arm::{ArmCore, EmulatedFunction, EmulatedFunctionParam, ResultWriter, SvcId};
use wie_jvm_support::JvmSupport;
use wie_util::{Result, read_generic, write_generic, write_null_terminated_string_bytes};
use wie_wipi_c::{
    MethodImpl, WIPICContext, WIPICMethodBody, WIPICResult,
    api::{database, graphics, kernel, media, misc, net},
};

use context::LgtWIPICContext;

use crate::runtime::java::classes::net::wie::{CletWrapper, CletWrapperCard, CletWrapperContext};
use crate::runtime::{SVC_CATEGORY_WIPIC, svc_ids::WIPICSvcId};

const TIME_VALUE_PTR: u32 = 0x7fff1004;

struct WIPICMethodResult {
    result: WIPICResult,
}

impl ResultWriter<WIPICMethodResult> for WIPICMethodResult {
    fn write(self, core: &mut ArmCore, next_pc: u32) -> Result<()> {
        core.write_return_value(&self.result.results)?;
        core.set_next_pc(next_pc)?;

        Ok(())
    }
}

struct CMethodProxy {
    context: LgtWIPICContext,
    body: WIPICMethodBody,
}

/// First SVC id of the WIPI net block on LGT, and how many positional slots it has.
///
/// ── WHY THIS IS A RANGE AND NOT THREE ENUM VARIANTS ──────────────────────────
/// `svc_ids.rs` named exactly three net slots (`Connect` `0x258`, `Close` `0x259`,
/// `SocketClose` `0x25e`) and `WIPICSvcId::try_from` turns every other id into a
/// `FatalError`. That is not "the network feature is missing" — it is **the emulator
/// aborting** the moment the game asks for a socket, which is the first thing it does
/// after `MC_netConnect`'s callback reports success. Hero3's private-server features
/// (네트워크 창고 / 복구 / 캐시샵) therefore could not work on an LGT boot at all,
/// while the identical code path works on KTF (MIGRATION_LGT.md §2.14).
///
/// The block is POSITIONAL and the numbering is the same one `wie_ktf`'s method table
/// is built on, which those three ids prove rather than assume: `Connect` is slot 0,
/// `Close` slot 1 and `SocketClose` **slot 6** — `0x258 + 6` — and slot 6 is exactly
/// where `net::socket_close` sits in the KTF table. Three points, one of them
/// non-adjacent, all consistent.
///
/// The bodies come from `wie_wipi_c::api::net`, which is carrier-agnostic: it talks to
/// `Platform::network()`, not to anything KTF-specific. So this is a MAPPING, not a
/// port — nothing about the socket layer had to be rewritten.
const NET_SVC_BASE: u32 = 0x258;
const NET_SVC_SLOTS: u32 = 34;

/// ★★ The SOCKET half of the net API lives in its OWN id group on LGT (2026-08-11,
/// device-measured). Mapping the 0x258 block was necessary but not sufficient: with it in
/// place the game got past `MC_netConnect`'s success callback and then died on
///
///     FatalError("Unknown LGT WIPIC SVC id 2000")   R0: 0x2  R1: 0x1
///
/// `2000` is `0x7d0`, nowhere near 0x258 — and `(r0, r1) = (2, 1)` is
/// `socket(AF_INET, SOCK_STREAM)`, which is exactly `wie_ktf`'s positional **slot 2**,
/// the first thing the game does once the connection is up. So the socket calls are a
/// separate decimal group whose first member is KTF slot 2:
///
///     LGT svc id 0x7d0 + k  ==  wie_ktf net slot 2 + k
///
/// Everything past the ones KTF verified stays a `trace_stub`, so if this group turns out
/// to be ordered differently the log names the id instead of the emulator aborting — the
/// failure mode that hid this for a whole session.
const SOCK_SVC_BASE: u32 = 0x7d0;
/// KTF slots 2..=33 → 32 ids.
const SOCK_SVC_SLOTS: u32 = 32;

/// One net slot's body, positionally identical to `wie_ktf`'s table so the two carriers
/// cannot drift. Slots marked VERIFIED there were reverse engineered from the game's own
/// calls; the rest stay logging stubs whose return values keep the guest walking forward
/// so a device log shows the whole sequence (that is what `net::trace_stub` is for).
fn net_slot_body(slot: u32) -> WIPICMethodBody {
    match slot {
        0 => net::connect.into_body(),
        1 => net::close.into_body(),
        2 => net::socket.into_body(), // VERIFIED socket-create (af, type)
        // ★ LGT connects HERE, not at slot 30 like KTF (MIGRATION_LGT.md §2.18). While
        // this was a stub the socket never actually connected, the guest sat waiting for
        // its completion callback, and the player saw a loading screen that never ended.
        3 => net::connect_lgt.into_body(),
        // ★ LGT sends and receives HERE, not at KTF's slots 31/32 (MIGRATION_LGT.md
        //   §2.18). Device trace once slot 3 really connected:
        //     slot4 (fd=4, buf=0x4013a554, len=6, …)   a1 = 06 00 00 00 00 0a
        //     slot5 (fd=4, buf=0x4013a144, max=0x400, …)
        //   The FIRST THREE arguments are exactly KTF's `(fd, buf, len)` / `(fd, buf,
        //   max)`, so the shared bodies drop straight in; the trailing words the stub
        //   printed are the dispatcher's fixed nine-argument frame, not parameters.
        //   While these were stubs the login packet was written into nothing and every
        //   read returned 0 bytes, so the game sat on "서버 응답 기다리는중" forever.
        4 => net::socket_send.into_body(),
        5 => net::socket_recv.into_body(),
        6 => net::socket_close.into_body(),
        7 => net::trace_stub(7, "MC_netSocketBind?", 0),
        8 => net::trace_stub(8, "MC_netGetMaxPacketLength?", 1500),
        9 => net::trace_stub(9, "MC_netSocketSendTo?", 0),
        10 => net::trace_stub(10, "MC_netSocketRcvFrom?", 0),
        11 => net::trace_stub(11, "MC_netGetHostAddr?", 0),
        12 => net::trace_stub(12, "MC_netSocketAccept?", -1),
        // VERIFIED: setReadCB(fd, cb, param). Must really fire, or the guest stops
        // polling recv and never sees server data.
        13 => net::set_read_cb.into_body(),
        14 => net::trace_stub(14, "MC_netSetWriteCB?", 0),
        15 => net::trace_stub(15, "MC_netHttpOpen?", -1),
        16 => net::trace_stub(16, "MC_netHttpConnect?", -1),
        17 => net::trace_stub(17, "MC_netHttpSetRequestMethod?", -1),
        18 => net::trace_stub(18, "MC_netHttpGetRequestMethod?", -1),
        19 => net::trace_stub(19, "MC_netHttpSetRequestProperty?", -1),
        20 => net::trace_stub(20, "MC_netHttpGetRequestProperty?", -1),
        21 => net::trace_stub(21, "MC_netHttpSetProxy?", -1),
        22 => net::trace_stub(22, "MC_netHttpGetProxy?", -1),
        23 => net::trace_stub(23, "MC_netHttpGetResponseCode?", -1),
        24 => net::trace_stub(24, "MC_netHttpGetResponseMessage?", -1),
        25 => net::trace_stub(25, "MC_netHttpGetHeaderField?", -1),
        26 => net::trace_stub(26, "MC_netHttpGetLength?", -1),
        27 => net::trace_stub(27, "MC_netHttpGetType?", -1),
        28 => net::trace_stub(28, "MC_netHttpGetEncoding?", -1),
        29 => net::trace_stub(29, "MC_netHttpClose?", -1),
        // VERIFIED: named connect (fd, addr, port, name, cb, param).
        30 => net::connect_named.into_body(),
        31 => net::socket_send.into_body(),     // VERIFIED: send(fd, buf, len)
        32 => net::socket_recv.into_body(),     // VERIFIED: recv(fd, buf, max), -19 = nothing yet
        33 => net::socket_close_fd.into_body(), // VERIFIED: close(fd); teardown is 33 then 1
        other => net::trace_stub(other, "MC_netUnk", -1),
    }
}

async fn handle_wipic_svc(core: &mut ArmCore, (system, jvm): &mut (System, Jvm), id: SvcId) -> Result<()> {
    let wipic_context = LgtWIPICContext::new(core.clone(), system.clone(), jvm.clone());
    let (_, lr) = core.read_pc_lr()?;
    // ★ The net block is dispatched BY RANGE, ahead of the named-id table: an id in it
    //   must never reach `try_from`, whose `_ =>` arm is a FatalError that kills the
    //   emulator. `svc_ids.rs` still names slots 0/1/6 and those arms are simply
    //   unreachable now — kept as the documentation of where the base came from.
    let net_slot = if (NET_SVC_BASE..NET_SVC_BASE + NET_SVC_SLOTS).contains(&id.0) {
        Some(id.0 - NET_SVC_BASE)
    } else if (SOCK_SVC_BASE..SOCK_SVC_BASE + SOCK_SVC_SLOTS).contains(&id.0) {
        Some(2 + (id.0 - SOCK_SVC_BASE))
    } else {
        None
    };
    if let Some(slot) = net_slot {
        let body = net_slot_body(slot);
        return EmulatedFunction::call(
            &CMethodProxy {
                context: wipic_context,
                body,
            },
            core,
            &mut (),
        )
        .await?
        .write(core, lr);
    }
    // ★ The 900 id group is net byte-order/resolver utilities (MIGRATION_LGT.md
    //   §2.18-(1')). Identified STATICALLY, no device round trip: the whole import
    //   surface sits in `.data` as 16-byte ARM stubs (`str lr,[sp,#-4]!; bl resolver;
    //   .word table; .word index`) — `0x14005e4` is (0x1fb, 901) and `0x14005f4` is
    //   (0x1fb, 904) — and the connect callback at `0x18900` shows both uses: 901's
    //   u16-truncated return becomes slot 3's `port` (= htons), 904's return becomes
    //   slot 3's `addr` (= inet_addr of "210.222.18.28"/"210.222.18.8"). Only 901 and
    //   904 appear in the stub table, so only they get bodies; any sibling id still
    //   falls through to the logging stub below and names itself in one run.
    let util_body = match id.0 {
        901 => Some(net::htons.into_body()),
        904 => Some(net::inet_addr.into_body()),
        _ => None,
    };
    if let Some(body) = util_body {
        return EmulatedFunction::call(
            &CMethodProxy {
                context: wipic_context,
                body,
            },
            core,
            &mut (),
        )
        .await?
        .write(core, lr);
    }
    // ★★ AN UNKNOWN WIPI SVC ID MUST NOT KILL THE APP (2026-08-11).
    //
    // `WIPICSvcId::try_from`'s `_` arm is a `FatalError`, and propagating it here cost
    // three build-and-test round trips in one session: the game's network bring-up walks
    // through SEVERAL id groups that LGT scatters across decimal ranges — 0x258 (600) for
    // connect/close, 0x7d0 (2000) for the socket calls, and now 0x385 (901) — and each
    // unmapped one aborted the emulator at the first call instead of reporting itself.
    // One id per round trip is not a workable way to enumerate an API.
    //
    // A logging stub is strictly better than dying: the guest keeps walking, ONE device
    // run names every id it still needs, and the failure that remains is a wrong return
    // value rather than a dead app. This is the same stance `net::trace_stub` already
    // takes for the net slots whose contracts are not verified yet.
    let method = match WIPICSvcId::try_from(id) {
        Ok(known) => known,
        Err(_) => {
            let body = net::trace_stub(id.0, "MC_wipicUnknown", 0);
            return EmulatedFunction::call(
                &CMethodProxy {
                    context: wipic_context,
                    body,
                },
                core,
                &mut (),
            )
            .await?
            .write(core, lr);
        }
    };
    let method = match method {
        WIPICSvcId::CletRegister => {
            return EmulatedFunction::call(&clet_register, core, jvm).await?.write(core, lr);
        }
        WIPICSvcId::GetFramebufferPointer => graphics::get_framebuffer_pointer.into_body(),
        WIPICSvcId::GetFramebufferWidth => graphics::get_framebuffer_width.into_body(),
        WIPICSvcId::GetFramebufferHeight => graphics::get_framebuffer_height.into_body(),
        WIPICSvcId::GetFramebufferBpl => graphics::get_framebuffer_bpl.into_body(),
        WIPICSvcId::GetFramebufferBpp => graphics::get_framebuffer_bpp.into_body(),
        WIPICSvcId::Printk => kernel::printk.into_body(),
        WIPICSvcId::Sprintk => kernel::sprintk.into_body(),
        WIPICSvcId::Unk13 => unk13.into_body(),
        WIPICSvcId::Unk1 => unk1.into_body(),
        WIPICSvcId::Exit => kernel::exit.into_body(),
        WIPICSvcId::Alloc => kernel::alloc.into_body(),
        WIPICSvcId::Calloc => kernel::calloc.into_body(),
        WIPICSvcId::Free => kernel::free.into_body(),
        WIPICSvcId::GetTotalMemory => kernel::get_total_memory.into_body(),
        WIPICSvcId::GetFreeMemory => kernel::get_free_memory.into_body(),
        WIPICSvcId::DefTimer => kernel::def_timer.into_body(),
        WIPICSvcId::SetTimer => kernel::set_timer.into_body(),
        WIPICSvcId::UnsetTimer => kernel::unset_timer.into_body(),
        WIPICSvcId::CurrentTime => kernel::current_time.into_body(),
        WIPICSvcId::GetSystemProperty => kernel::get_system_property.into_body(),
        WIPICSvcId::SetSystemProperty => kernel::set_system_property.into_body(),
        WIPICSvcId::GetResourceId => kernel::get_resource_id.into_body(),
        WIPICSvcId::GetResource => kernel::get_resource.into_body(),
        WIPICSvcId::Unk2 => unk2.into_body(),
        WIPICSvcId::GetImageProperty => graphics::get_image_property.into_body(),
        WIPICSvcId::GetImageFramebuffer => graphics::get_image_framebuffer.into_body(),
        WIPICSvcId::GetScreenFramebuffer => graphics::get_screen_framebuffer.into_body(),
        WIPICSvcId::DestroyOffscreenFramebuffer => graphics::destroy_offscreen_framebuffer.into_body(),
        WIPICSvcId::CreateOffscreenFramebuffer => graphics::create_offscreen_framebuffer.into_body(),
        WIPICSvcId::InitContext => graphics::init_context.into_body(),
        WIPICSvcId::SetContext => graphics::set_context.into_body(),
        WIPICSvcId::PutPixel => graphics::put_pixel.into_body(),
        WIPICSvcId::DrawLine => graphics::draw_line.into_body(),
        WIPICSvcId::DrawRect => graphics::draw_rect.into_body(),
        WIPICSvcId::FillRect => graphics::fill_rect.into_body(),
        WIPICSvcId::CopyFrameBuffer => graphics::copy_frame_buffer.into_body(),
        WIPICSvcId::DrawImage => graphics::draw_image.into_body(),
        WIPICSvcId::CopyArea => graphics::copy_area.into_body(),
        WIPICSvcId::DrawString => graphics::draw_string.into_body(),
        WIPICSvcId::GetRgbPixels => graphics::get_rgb_pixels.into_body(),
        WIPICSvcId::SetRgbPixels => graphics::set_rgb_pixels.into_body(),
        WIPICSvcId::FlushLcd => graphics::flush_lcd.into_body(),
        WIPICSvcId::GetPixelFromRgb => graphics::get_pixel_from_rgb.into_body(),
        WIPICSvcId::GetRgbFromPixel => graphics::get_rgb_from_pixel.into_body(),
        WIPICSvcId::GetDisplayInfo => graphics::get_display_info.into_body(),
        WIPICSvcId::Repaint => graphics::repaint.into_body(),
        WIPICSvcId::GetFont => graphics::get_font.into_body(),
        WIPICSvcId::GetFontHeight => graphics::get_font_height.into_body(),
        WIPICSvcId::GetFontAscent => graphics::get_font_ascent.into_body(),
        WIPICSvcId::GetFontDescent => graphics::get_font_descent.into_body(),
        WIPICSvcId::GetStringWidth => graphics::get_string_width.into_body(),
        WIPICSvcId::CreateImage => graphics::create_image.into_body(),
        WIPICSvcId::Unk0 => unk0.into_body(),
        WIPICSvcId::Unk11 => unk11.into_body(),
        WIPICSvcId::Unk3 => unk3.into_body(),
        WIPICSvcId::Unk4 => unk4.into_body(),
        WIPICSvcId::Unk7 => unk7.into_body(),
        WIPICSvcId::Unk6 => unk6.into_body(),
        WIPICSvcId::TimeNow => time_now.into_body(),
        WIPICSvcId::TimeComponent => time_component.into_body(),
        WIPICSvcId::TimeConvert => time_convert.into_body(),
        WIPICSvcId::TimeToTm => time_to_tm.into_body(),
        WIPICSvcId::DateTimeToTm => time_to_tm.into_body(),
        WIPICSvcId::OpenDatabase => database::open_database.into_body(),
        WIPICSvcId::ReadRecordSingle => database::stream_read.into_body(),
        WIPICSvcId::WriteRecordSingle => database::stream_write.into_body(),
        WIPICSvcId::CloseDatabase => database::close_database.into_body(),
        WIPICSvcId::Unk12 => database::seek_record_single.into_body(),
        WIPICSvcId::Unk9 => database::list_record_info.into_body(),
        WIPICSvcId::DeleteRecord => database::delete_database.into_body(),
        WIPICSvcId::ListRecord => database::list_record.into_body(),
        WIPICSvcId::UpdateRecord => database::update_record.into_body(),
        WIPICSvcId::SelectRecord => database::select_record.into_body(),
        WIPICSvcId::Unk8 => database::exists_database.into_body(),
        WIPICSvcId::Connect => net::connect.into_body(),
        WIPICSvcId::Close => net::close.into_body(),
        WIPICSvcId::SocketClose => net::socket_close.into_body(),
        WIPICSvcId::ClipCreate => media::clip_create.into_body(),
        WIPICSvcId::ClipFree => media::clip_free.into_body(),
        WIPICSvcId::ClipPutData => media::clip_put_data.into_body(),
        WIPICSvcId::Unk15 => unk15.into_body(),
        WIPICSvcId::ClipGetVolume => media::clip_get_volume.into_body(),
        WIPICSvcId::ClipSetVolume => media::clip_set_volume.into_body(),
        WIPICSvcId::Play => media::play.into_body(),
        WIPICSvcId::Stop => media::stop.into_body(),
        WIPICSvcId::Unk5 => unk5.into_body(),
        WIPICSvcId::Vibrator => media::vibrator.into_body(),
        WIPICSvcId::Unk14 => unk14.into_body(),
        WIPICSvcId::ClipAllocPlayer => media::clip_alloc_player.into_body(),
        WIPICSvcId::ClipFreePlayer => media::clip_free_player.into_body(),
        WIPICSvcId::Unk10 => unk10.into_body(),
        WIPICSvcId::SetMuteState => media::set_mute_state.into_body(),
        WIPICSvcId::GetMuteState => media::get_mute_state.into_body(),
        WIPICSvcId::BackLight => misc::back_light.into_body(),
    };

    EmulatedFunction::call(
        &CMethodProxy {
            context: wipic_context,
            body: method,
        },
        core,
        &mut (),
    )
    .await?
    .write(core, lr)
}

#[async_trait::async_trait]
impl EmulatedFunction<(), WIPICMethodResult, ()> for CMethodProxy {
    async fn call(&self, core: &mut ArmCore, _: &mut ()) -> Result<WIPICMethodResult> {
        let a0 = u32::get(core, 0);
        let a1 = u32::get(core, 1);
        let a2 = u32::get(core, 2);
        let a3 = u32::get(core, 3);
        let a4 = u32::get(core, 4);
        let a5 = u32::get(core, 5);
        let a6 = u32::get(core, 6);
        let a7 = u32::get(core, 7);
        let a8 = u32::get(core, 8);

        let result = self
            .body
            .call(&mut self.context.clone(), vec![a0, a1, a2, a3, a4, a5, a6, a7, a8].into_boxed_slice())
            .await?;

        Ok(WIPICMethodResult { result })
    }
}

pub fn register_wipic_svc_handler(core: &mut ArmCore, system: &System, jvm: &Jvm) -> Result<()> {
    core.register_svc_handler(SVC_CATEGORY_WIPIC, handle_wipic_svc, &(system.clone(), jvm.clone()))
}

async fn clet_register(core: &mut ArmCore, jvm: &mut Jvm, function_table: u32, a1: u32) -> Result<()> {
    tracing::debug!("clet_register({function_table:#x}, {a1:#x})");

    let functions: CletFunctions = read_generic(core, function_table)?;

    let context = CletWrapperContext { core: core.clone() };
    let clet_wrapper_class = ClassDefinitionImpl::from_class_proto(CletWrapper::as_proto(), Box::new(context.clone()) as Box<_>);
    let clet_wrapper_card_class = ClassDefinitionImpl::from_class_proto(CletWrapperCard::as_proto(), Box::new(context) as Box<_>);
    jvm.register_class(Box::new(clet_wrapper_class), None).await.unwrap();
    jvm.register_class(Box::new(clet_wrapper_card_class), None).await.unwrap();

    jvm.put_static_field("net/wie/CletWrapper", "startClet", "I", functions.start_clet as i32)
        .await
        .unwrap();
    jvm.put_static_field("net/wie/CletWrapper", "pauseClet", "I", functions.pause_clet as i32)
        .await
        .unwrap();
    jvm.put_static_field("net/wie/CletWrapper", "resumeClet", "I", functions.resume_clet as i32)
        .await
        .unwrap();
    jvm.put_static_field("net/wie/CletWrapper", "destroyClet", "I", functions.destroy_clet as i32)
        .await
        .unwrap();
    jvm.put_static_field("net/wie/CletWrapper", "paintClet", "I", functions.paint_clet as i32)
        .await
        .unwrap();
    jvm.put_static_field("net/wie/CletWrapper", "handleCletEvent", "I", functions.handle_clet_event as i32)
        .await
        .unwrap();

    let main_class_name = JavaLangString::from_rust_string(jvm, "net/wie/CletWrapper").await.unwrap();
    let mut args_array = jvm.instantiate_array("Ljava/lang/String;", 1).await.unwrap();
    jvm.store_array(&mut args_array, 0, vec![main_class_name]).await.unwrap();

    let result: JvmResult<()> = jvm
        .invoke_static("org/kwis/msp/lcdui/Main", "main", "([Ljava/lang/String;)V", (args_array,))
        .await;

    if let Err(x) = result {
        return Err(JvmSupport::to_wie_err(jvm, x).await);
    }

    Ok(())
}

async fn unk0(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk0({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    // graphics

    Ok(0)
}

async fn unk1(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk1({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    // kernel

    Ok(0)
}

async fn unk2(context: &mut dyn WIPICContext) -> Result<u32> {
    tracing::warn!("stub unk2");

    // OEMC_knlGetProgramInfo? get app id
    let app_id = context.system().aid().to_string();
    let result = context.alloc_raw((app_id.len() + 1) as u32)?;
    write_null_terminated_string_bytes(context, result, app_id.as_bytes())?;

    Ok(result)
}

async fn unk3(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk3({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    Ok(0)
}

async fn unk4(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk4({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    Ok(0)
}

async fn unk5(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk5({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    // media

    Ok(0)
}

async fn unk6(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk6({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    Ok(0)
}

async fn unk7(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk7({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    Ok(0)
}

async fn time_now(context: &mut dyn WIPICContext, component_class: u32) -> Result<u32> {
    let epoch_seconds = context.system().platform().now().raw() / 1000;
    tracing::debug!("LGT_timeNow({component_class:#x}) -> {epoch_seconds}");

    write_time_value(context, epoch_seconds as u32)
}

async fn time_component(_context: &mut dyn WIPICContext, name: u32) -> Result<u32> {
    tracing::debug!("LGT_timeComponent({name:#x})");

    Ok(name)
}

async fn time_convert(context: &mut dyn WIPICContext, date_time: u32, component: u32) -> Result<u32> {
    tracing::debug!("LGT_timeConvert({date_time:#x}, {component:#x})");

    let timestamp = read_time_value(context, date_time)?;
    write_time_value(context, timestamp)
}

async fn time_to_tm(context: &mut dyn WIPICContext, time_value: u32, out_ptr: u32) -> Result<i32> {
    tracing::debug!("LGT_timeToTm({time_value:#x}, {out_ptr:#x})");

    let timestamp = read_time_value(context, time_value)?;
    let (year, month, day, hour, minute, second) = unix_seconds_to_utc(timestamp as i64);
    write_generic(context, out_ptr, second)?;
    write_generic(context, out_ptr + 4, minute)?;
    write_generic(context, out_ptr + 8, hour)?;
    write_generic(context, out_ptr + 12, day)?;
    write_generic(context, out_ptr + 16, month - 1)?;
    write_generic(context, out_ptr + 20, year - 1900)?;

    Ok(0)
}

fn write_time_value(context: &mut dyn WIPICContext, timestamp: u32) -> Result<u32> {
    let time_value_ptr: u32 = read_generic(context, TIME_VALUE_PTR)?;
    let memory = if time_value_ptr != 0 {
        WIPICIndirectPtr(time_value_ptr)
    } else {
        let memory = context.alloc(4)?;
        write_generic(context, TIME_VALUE_PTR, memory.0)?;
        memory
    };
    write_generic(context, context.data_ptr(memory)?, timestamp)?;
    Ok(memory.0)
}

fn read_time_value(context: &mut dyn WIPICContext, handle: u32) -> Result<u32> {
    read_generic(context, context.data_ptr(WIPICIndirectPtr(handle))?)
}

fn unix_seconds_to_utc(timestamp: i64) -> (i32, i32, i32, i32, i32, i32) {
    let days = timestamp.div_euclid(86_400);
    let seconds_of_day = timestamp.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = (seconds_of_day / 3600) as i32;
    let minute = ((seconds_of_day % 3600) / 60) as i32;
    let second = (seconds_of_day % 60) as i32;

    (year, month, day, hour, minute, second)
}

fn civil_from_days(days: i64) -> (i32, i32, i32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_param = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_param + 2) / 5 + 1;
    let month = month_param + if month_param < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };

    (year as i32, month as i32, day as i32)
}

async fn unk10(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk10({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    Ok(0)
}

async fn unk11(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk11({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    Ok(0)
}

async fn unk13(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk13({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    // kernel

    Ok(0)
}

async fn unk14(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk14({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    // media

    Ok(0)
}

async fn unk15(_context: &mut dyn WIPICContext, a0: u32, a1: u32, a2: u32, a3: u32) -> Result<u32> {
    tracing::warn!("stub unk15({a0:#x}, {a1:#x}, {a2:#x}, {a3:#x})");

    // media

    Ok(0)
}
