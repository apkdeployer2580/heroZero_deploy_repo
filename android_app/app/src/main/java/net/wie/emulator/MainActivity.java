package net.wie.emulator;

import android.annotation.SuppressLint;
import android.app.AlertDialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.hardware.input.InputManager;
import android.media.AudioManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.view.InputDevice;
import android.view.MotionEvent;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.widget.Toast;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;

import androidx.core.content.FileProvider;

import com.google.androidgamesdk.GameActivity;

/**
 * Thin GameActivity subclass. All app logic is in the Rust cdylib (libmain.so), entered through
 * android_main(). The static loadLibrary, the manifest's android.app.lib_name and Cargo's
 * [lib] name all say "main".
 *
 * ⛔ Do not move this class out of net.wie.emulator: hz_android/src/java.rs finds it BY NAME over
 * JNI, and a moved class fails only at call time (orientation, the dialogs, exit go dead).
 */
public class MainActivity extends GameActivity implements InputManager.InputDeviceListener {
    static {
        System.loadLibrary("main");
    }

    // ndk_context hands native code the Application, not the Activity, so the instance is kept
    // here for the static entry points below.
    private static MainActivity instance;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        if (instance != null) {
            // A second instance in this process (singleTask leaves only a task-clearing start):
            // GameActivity.onCreate would terminateNativeCode the first one's game, which blocks
            // for good like onDestroy below (S25 2026-10-27: an ANR in exactly that call). End the
            // process instead; the next launch boots fresh.
            System.exit(0);
        }
        instance = this;
        super.onCreate(savedInstanceState);
        // The game surface holds the window's focus (keys, IME), and a focused view draws Android's
        // default focus highlight whenever its window is out of touch mode — a #29FFFFFF veil over
        // the whole picture that nothing redraws away, since the game draws into the Surface, not
        // the view (S25 2026-10-27: twice, after the import's file picker was closed with back).
        mSurfaceView.setDefaultFocusHighlightEnabled(false);
        // Volume keys always drive MEDIA volume, not the ringer, even while the game is silent.
        setVolumeControlStream(AudioManager.STREAM_MUSIC);
        letContentUseTheCutout();
        hideSystemBars();
        // A pad that goes away with its stick leaning sends no resting sample (below).
        InputManager input = (InputManager) getSystemService(INPUT_SERVICE);
        if (input != null) {
            input.registerInputDeviceListener(this, null);
        }
    }

    /**
     * The picture under the display cutout (notch / punch-hole) too (C2, hero3's — its player
     * 2026-08-19: "스마트폰 전체화면에 게임화면이 꽉차게"): GameActivity's surface is the WINDOW's
     * size, and with the default policy a landscape window stops at the cutout's safe area, leaving
     * a black bar along that edge. The guest canvas is sized from the window at boot
     * (hz_android guest_size), so the picture takes the whole screen. A window attribute set once —
     * the system does not clear it the way it clears the bar flags {@link #hideSystemBars} re-sets.
     * Cutouts exist from API 28; ALWAYS (API 30) covers every edge, SHORT_EDGES is what 28 can say.
     */
    private void letContentUseTheCutout() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) {
            return;
        }
        final Window w = getWindow();
        WindowManager.LayoutParams lp = w.getAttributes();
        lp.layoutInDisplayCutoutMode = (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R)
                ? WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
                : WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
        w.setAttributes(lp);
    }

    /**
     * Full immersive: status AND navigation bars hidden, revealed transiently by an edge swipe.
     * Re-asserted on every focus gain because the system clears it whenever the bars show.
     */
    private void hideSystemBars() {
        final Window w = getWindow();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            w.setDecorFitsSystemWindows(false);
            WindowInsetsController c = w.getInsetsController();
            if (c != null) {
                c.hide(WindowInsets.Type.systemBars());
                c.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            w.getDecorView().setSystemUiVisibility(
                    View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                            | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                            | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                            | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                            | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                            | View.SYSTEM_UI_FLAG_FULLSCREEN);
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
        }
    }

    // ── The back key and the activity's end ──────────────────────────────────────────
    //
    // The phone's back key (or back gesture) is the GAME's back: GameActivity.onKeyDown hands it to
    // the native glue and hz_android/src/keys.rs sends RSK. The manifest keeps that path (predictive
    // back off) and takes every configuration change in place, so nothing should end this activity
    // but the game's own 「종료」 (quitApp).

    /** Reached only if the native side ever declines the back key: it must not finish the game. */
    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
    }

    /**
     * An activity that ends while the process lives takes the process with it, like quitApp.
     * GameActivity.onDestroy waits (terminateNativeCode → the glue's APP_CMD_DESTROY) for
     * android_main to return, and winit never ends its loop on Destroy — so super.onDestroy() would
     * block the main thread for good and the next launch in this process would hang on the splash
     * (S25 2026-10-27, after a back key finished the activity). Every save write is already on disk
     * (database::write_atomic), and the next launch boots fresh.
     */
    @Override
    @SuppressLint("MissingSuperCall")
    protected void onDestroy() {
        System.exit(0);
    }

    // ── Gamepad sticks ────────────────────────────────────────────────────────────
    //
    // Captured here, at the top of the dispatch chain, because the GameActivity native input
    // path cannot carry them: android_native_app_glue's default motion filter drops every
    // non-touchscreen MotionEvent, and winit would read a survivor as a touch (hero2, read
    // end-to-end). Buttons and the D-pad DO reach winit as key events and are mapped there
    // (hz_android/src/keys.rs); only the axes take this route. The source is matched with the
    // full mask so a mouse or a keyboard never ends up here.

    /** The devices whose stick motion reached the game (main thread only). */
    private final Set<Integer> padsSeen = new HashSet<>();

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent e) {
        if ((e.getSource() & InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK
                && e.getAction() == MotionEvent.ACTION_MOVE) {
            padsSeen.add(e.getDeviceId());
            // Left stick + hat (most pads report the D-pad as hat axes) + triggers (which many
            // Bluetooth pads report only as axes: LTRIGGER/RTRIGGER or BRAKE/GAS, so take the
            // larger). Only the latest position matters to a 4-way latch.
            nativeGamepadAxes(
                    e.getAxisValue(MotionEvent.AXIS_X), e.getAxisValue(MotionEvent.AXIS_Y),
                    e.getAxisValue(MotionEvent.AXIS_HAT_X), e.getAxisValue(MotionEvent.AXIS_HAT_Y),
                    Math.max(e.getAxisValue(MotionEvent.AXIS_LTRIGGER),
                            e.getAxisValue(MotionEvent.AXIS_BRAKE)),
                    Math.max(e.getAxisValue(MotionEvent.AXIS_RTRIGGER),
                            e.getAxisValue(MotionEvent.AXIS_GAS)));
            return true;
        }
        return super.dispatchGenericMotionEvent(e);
    }

    /** Implemented in Rust (hz_android/src/lib.rs, resolved by JNI symbol name). */
    private static native void nativeGamepadAxes(
            float x, float y, float hatX, float hatY, float lt, float rt);

    /**
     * A pad switched off or out of range: Android cancels its held buttons (they reach the game as
     * releases) but sends no resting stick sample, so a stick, hat or trigger held at that moment
     * would walk on for good. Rust lets go of everything the pad held (App::pad_lost).
     */
    @Override
    public void onInputDeviceRemoved(int deviceId) {
        if (padsSeen.remove(deviceId)) {
            nativeGamepadLost();
        }
    }

    @Override
    public void onInputDeviceAdded(int deviceId) {}

    @Override
    public void onInputDeviceChanged(int deviceId) {}

    /** Implemented in Rust (hz_android/src/lib.rs). */
    private static native void nativeGamepadLost();

    /** From native code. ActivityInfo.SCREEN_ORIENTATION_* — must run on the UI thread. */
    public static void requestOrientation(final int orientation) {
        final MainActivity a = instance;
        if (a == null) {
            return;
        }
        a.runOnUiThread(() -> a.setRequestedOrientation(orientation));
    }

    // ── 진단 정보 · 세이브 관리 (B3, 2026-10-26 — hero2's dialogs) ─────────────────────────────
    //
    // Settings › 진단 정보 is the platform's dialog, as in hero2 and hero3: what the host knows, a
    // 복사 for a tester without adb, and 세이브 관리 — the vault's restore points and the export.
    // Every decision about saves is native (hz_android/src/saves.rs, tested there); this side only
    // shows its lists and sentences and passes the picked key back. A restore is APPLIED AT THE NEXT
    // LAUNCH, before the game opens a store (the running game holds the character in its own heap
    // and would write it back), so the dialog offers to close the app.

    /** Implemented in Rust (hz_android/src/lib.rs `saves_jni`). */
    private static native String nativeRestoreList(String filesDir);

    private static native boolean nativeRestoreRequest(String filesDir, String key);

    private static native String nativeSaveExport(String filesDir, String extDir);

    /** From native code (settings › 진단 정보): Java's facts above the native ones. */
    @SuppressWarnings("deprecation")
    public static void showDiagnostics(final String rustInfo) {
        final MainActivity a = instance;
        if (a == null) {
            return;
        }
        a.runOnUiThread(() -> {
            String ver;
            try {
                PackageInfo pi = a.getPackageManager().getPackageInfo(a.getPackageName(), 0);
                long code = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P ? pi.getLongVersionCode() : pi.versionCode;
                ver = pi.versionName + " (" + code + ")";
            } catch (Exception e) {
                ver = "?";
            }
            final String text = "앱 " + ver
                    + "\nAndroid " + Build.VERSION.RELEASE + " (API " + Build.VERSION.SDK_INT + ")"
                    + "\n기기 " + Build.MANUFACTURER + " " + Build.MODEL
                    + "\n" + rustInfo;
            new AlertDialog.Builder(a)
                    .setTitle("진단 정보")
                    .setMessage(text + "\n\n문제가 있으면 [복사]를 눌러 개발자에게 붙여넣어 보내주세요.")
                    .setPositiveButton("복사", (d, w) -> {
                        ClipboardManager cm = (ClipboardManager) a.getSystemService(CLIPBOARD_SERVICE);
                        if (cm != null) {
                            cm.setPrimaryClip(ClipData.newPlainText("영웅서기 Zero 진단 정보", text));
                            toast(a, "복사되었습니다");
                        }
                    })
                    .setNegativeButton("닫기", null)
                    .setNeutralButton("세이브 관리", (d, w) -> showSaveMenu(a))
                    .show();
        });
    }

    /** 진단 정보 → 세이브 관리: hero2's three, and the folder copy for a bug report. */
    private static void showSaveMenu(final MainActivity a) {
        final String[] items = {
                "다른 기기로 보내기 (폰·PC, 내보내기)",
                "다른 기기에서 받기 (폰·PC, 가져오기)",
                "백업에서 복원",
                "세이브 폴더로 복사 (문제 신고용)",
        };
        new AlertDialog.Builder(a)
                .setTitle("세이브 관리")
                .setItems(items, (d, which) -> {
                    if (which == 0) {
                        showTransferExport(a);
                    } else if (which == 1) {
                        showTransferImport(a);
                    } else if (which == 2) {
                        showSaveRestore(a);
                    } else {
                        showSaveExport(a);
                    }
                })
                .setNegativeButton("닫기", null)
                .show();
    }

    // ── 세이브 옮기기 (B4, 2026-10-26 — hero2's) ───────────────────────────────────────────────
    //
    // 내보내기 makes ONE file (hz_android/src/save_transfer.rs), the player sends it however they like
    // (카카오톡 나와의 채팅, Quick Share, Drive, a cable), and 가져오기 on the other phone reads it. The
    // import is APPLIED ON THE NEXT LAUNCH for the restore's reason, after the native side has put
    // this phone's own saves into the vault (saves.rs). Java only moves bytes between the system
    // picker / share sheet and a file in the cache dir; what goes into the file, what is accepted and
    // how this phone's saves are kept first is decided natively, in one tested place.

    private static final int REQ_TRANSFER_EXPORT_FILE = 0x485a;
    private static final int REQ_TRANSFER_IMPORT_PICK = 0x485b;
    /** The share provider's authority after the package name — AndroidManifest.xml. */
    private static final String SAVE_SHARE_AUTHORITY = ".saveshare";
    /** cache/<this> holds the transfer file — res/xml/save_share_paths.xml serves exactly it. */
    private static final String SAVE_SHARE_DIR = "save_share";
    /** Above any transfer file (the native side refuses past 1 MB). */
    private static final long IMPORT_MAX_BYTES = 2L * 1024 * 1024;

    /** Implemented in Rust (lib.rs `saves_jni`). Each answers "ok…" or "err|<a sentence>". */
    private static native String nativeTransferExport(String filesDir, String outPath, int app);

    private static native String nativeTransferCheck(String filesDir, String inPath, int app);

    private static native String nativeTransferSchedule(String filesDir, String inPath, int app);

    private static void showTransferExport(final MainActivity a) {
        new AlertDialog.Builder(a)
                .setTitle("세이브 내보내기")
                .setMessage("이 폰의 세이브(슬롯 1~10)와 계정창고를 파일 하나로 만듭니다.\n\n"
                        + "⚠ 마지막으로 저장한 상태가 들어갑니다. 먼저 게임에서 저장하세요.\n\n"
                        + "[보내기] 카카오톡·Quick Share 등으로 바로 보냅니다.\n"
                        + "[파일로 저장] 이 폰의 폴더(다운로드 등)에 저장합니다.")
                .setPositiveButton("보내기", (d, w) -> shareExport(a))
                .setNeutralButton("파일로 저장", (d, w) -> pickExportFile(a))
                .setNegativeButton("취소", null)
                .show();
    }

    /** Make the transfer file in the cache dir; null (after saying why) when it could not be made. */
    private static File exportToCache(MainActivity a) {
        File dir = new File(a.getCacheDir(), SAVE_SHARE_DIR);
        // One file at a time: an older export left here is a stale copy nobody should send.
        File[] old = dir.listFiles();
        if (old != null) {
            for (File f : old) {
                //noinspection ResultOfMethodCallIgnored
                f.delete();
            }
        }
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        String name = "영웅서기제로_세이브_" + new SimpleDateFormat("yyyyMMdd_HHmm", Locale.KOREA).format(new Date()) + ".zip";
        File out = new File(dir, name);
        String r;
        try {
            r = nativeTransferExport(a.getFilesDir().getAbsolutePath(), out.getAbsolutePath(), appVersion(a));
        } catch (Throwable t) {
            r = null;
        }
        if (r == null || !r.startsWith("ok")) {
            showSaveResult(a, "내보내기 실패", reason(r, "세이브 파일을 만들지 못했습니다."));
            return null;
        }
        return out;
    }

    private static void shareExport(MainActivity a) {
        File out = exportToCache(a);
        if (out == null) {
            return;
        }
        try {
            Uri uri = FileProvider.getUriForFile(a, a.getPackageName() + SAVE_SHARE_AUTHORITY, out);
            Intent send = new Intent(Intent.ACTION_SEND);
            send.setType("application/zip");
            send.putExtra(Intent.EXTRA_STREAM, uri);
            // ClipData too: since API 29 that is what carries the read grant through the chooser.
            send.setClipData(ClipData.newRawUri(out.getName(), uri));
            send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            a.startActivity(Intent.createChooser(send, "세이브 파일 보내기"));
        } catch (Throwable t) {
            showSaveResult(a, "보내기 실패", "공유 창을 열지 못했습니다. [파일로 저장]으로 저장한 뒤 보내 주세요.");
        }
    }

    private static void pickExportFile(MainActivity a) {
        // Made BEFORE the picker opens, so a failure is said here — not after the player has made
        // an empty file somewhere.
        File out = exportToCache(a);
        if (out == null) {
            return;
        }
        Intent pick = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        pick.addCategory(Intent.CATEGORY_OPENABLE);
        pick.setType("application/zip");
        pick.putExtra(Intent.EXTRA_TITLE, out.getName());
        try {
            a.startActivityForResult(pick, REQ_TRANSFER_EXPORT_FILE);
        } catch (Throwable t) {
            showSaveResult(a, "파일로 저장 실패", "파일 저장 창을 열지 못했습니다. [보내기]를 써 주세요.");
        }
    }

    /** The picker came back with a place to save: copy the file made before it opened. */
    private static void finishExportFile(MainActivity a, Uri dest) {
        // The newest file there — the system may have recreated the process while the picker was
        // open, and the cache survives that. Made again when it did not.
        File src = newestFile(new File(a.getCacheDir(), SAVE_SHARE_DIR));
        if (src == null) {
            src = exportToCache(a);
            if (src == null) {
                return;
            }
        }
        try (InputStream in = new FileInputStream(src);
             OutputStream out = a.getContentResolver().openOutputStream(dest)) {
            if (out == null) {
                throw new IOException("no output stream");
            }
            copy(in, out, Long.MAX_VALUE);
        } catch (Throwable t) {
            showSaveResult(a, "파일로 저장 실패", "파일을 쓰지 못했습니다. 다른 폴더를 골라 다시 해 보세요.");
            return;
        }
        showSaveResult(a, "저장했습니다", src.getName()
                + "\n\n이 파일을 다른 폰으로 보낸 뒤, 그 폰에서\n설정 → 진단 정보 → 세이브 관리 → 다른 기기에서 받기\n를 누르세요."
                + "\n\nPC 로 옮길 때는 PC 의 문서\\heroZero\\save_import 폴더에 넣고, PC 게임의 같은 메뉴에서 [다른 기기에서 받기]를 누르세요.");
    }

    private static void showTransferImport(final MainActivity a) {
        new AlertDialog.Builder(a)
                .setTitle("세이브 가져오기")
                .setMessage("다른 폰이나 PC 에서 내보낸 세이브 파일(폰: 영웅서기제로_세이브_….zip, PC: herozero_save_….zip)을 고르세요.\n\n"
                        + "⚠ 가져오면 이 폰의 세이브(슬롯 1~10)와 계정창고가 모두 파일 내용으로 바뀝니다. "
                        + "지금 이 폰의 세이브는 백업으로 남아 [백업에서 복원]으로 되돌릴 수 있습니다.")
                .setPositiveButton("파일 고르기", (d, w) -> {
                    Intent pick = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                    pick.addCategory(Intent.CATEGORY_OPENABLE);
                    // Any type: a file that came through a messenger is not always labelled a zip.
                    pick.setType("*/*");
                    try {
                        a.startActivityForResult(pick, REQ_TRANSFER_IMPORT_PICK);
                    } catch (Throwable t) {
                        showSaveResult(a, "가져오기 실패", "파일 선택 창을 열지 못했습니다.");
                    }
                })
                .setNegativeButton("취소", null)
                .show();
    }

    /** The picked file: copy it in, let the native side read it, and ask with what it found. */
    private static void importPicked(final MainActivity a, Uri src) {
        File dir = new File(a.getCacheDir(), "save_import");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        final File cand = new File(dir, "candidate.zip");
        try (InputStream in = a.getContentResolver().openInputStream(src);
             OutputStream out = new FileOutputStream(cand)) {
            if (in == null) {
                throw new IOException("no input stream");
            }
            copy(in, out, IMPORT_MAX_BYTES);
        } catch (Throwable t) {
            //noinspection ResultOfMethodCallIgnored
            cand.delete();
            showSaveResult(a, "가져오기 실패", "파일을 읽지 못했습니다. 영웅서기 Zero 세이브 파일인지 확인해 주세요.");
            return;
        }
        String r;
        try {
            r = nativeTransferCheck(a.getFilesDir().getAbsolutePath(), cand.getAbsolutePath(), appVersion(a));
        } catch (Throwable t) {
            r = null;
        }
        if (r == null || !r.startsWith("ok")) {
            //noinspection ResultOfMethodCallIgnored
            cand.delete();
            showSaveResult(a, "가져오기 실패", reason(r, "파일을 확인하지 못했습니다."));
            return;
        }
        // ok · created|ms · app|vc · file|<slot>|<ms>… · extra|storage · here|<slot>|<ms>… — saves::import_summary.
        final SimpleDateFormat fmt = new SimpleDateFormat("M월 d일 HH:mm", Locale.KOREA);
        long created = 0;
        StringBuilder file = new StringBuilder();
        StringBuilder here = new StringBuilder();
        boolean storage = false;
        for (String line : r.split("\n")) {
            String[] f = line.split("\\|");
            if (f.length < 2) {
                continue;
            }
            switch (f[0]) {
                case "created":
                    created = parseLong(f[1]);
                    break;
                case "file":
                case "here":
                    long ms = f.length > 2 ? parseLong(f[2]) : 0;
                    String one = "\n  슬롯 " + f[1] + (ms > 0 ? " · " + fmt.format(new Date(ms)) + " 저장" : "");
                    (f[0].equals("file") ? file : here).append(one);
                    break;
                case "extra":
                    storage = storage || f[1].equals("storage");
                    break;
                default:
                    break;
            }
        }
        StringBuilder msg = new StringBuilder("받은 파일");
        if (created > 0) {
            msg.append(" (").append(fmt.format(new Date(created))).append(" 내보냄)");
        }
        msg.append(":").append(file);
        if (storage) {
            msg.append("\n  + 계정창고");
        }
        msg.append("\n\n이 폰의 지금 세이브 (백업으로 보관됨):");
        msg.append(here.length() > 0 ? here.toString() : "\n  없음");
        msg.append("\n\n[가져오기]를 누르면 앱이 닫힙니다. 다시 실행하면 받은 세이브로 시작합니다.");
        new AlertDialog.Builder(a)
                .setTitle("이 세이브를 가져올까요?")
                .setMessage(msg.toString())
                .setPositiveButton("가져오기", (d, w) -> scheduleImport(a, cand))
                .setNegativeButton("취소", (d, w) -> cand.delete())
                .show();
    }

    private static void scheduleImport(MainActivity a, File cand) {
        String r;
        try {
            r = nativeTransferSchedule(a.getFilesDir().getAbsolutePath(), cand.getAbsolutePath(), appVersion(a));
        } catch (Throwable t) {
            r = null;
        }
        //noinspection ResultOfMethodCallIgnored
        cand.delete();
        if (r == null || !r.startsWith("ok")) {
            showSaveResult(a, "가져오기 실패", reason(r, "가져오기를 준비하지 못했습니다."));
            return;
        }
        // No 「나중에」 (the restore offers one): a save made before the relaunch would only be run
        // over by the import, so closing now is the one honest choice.
        new AlertDialog.Builder(a)
                .setTitle("가져오기 준비 완료")
                .setMessage("앱을 닫습니다.\n다시 실행하면 받은 세이브로 시작합니다.")
                .setPositiveButton("확인", (d, w) -> quitApp())
                .setCancelable(false)
                .show();
    }

    /** From native code: what the boot did with a scheduled import. The first line is the title. */
    public static void showSaveMessage(final String text) {
        final MainActivity a = instance;
        if (a == null || text == null) {
            return;
        }
        a.runOnUiThread(() -> {
            int nl = text.indexOf('\n');
            showSaveResult(a, nl > 0 ? text.substring(0, nl) : "세이브", nl > 0 ? text.substring(nl + 1) : text);
        });
    }

    private static void showSaveResult(MainActivity a, String title, String message) {
        new AlertDialog.Builder(a)
                .setTitle(title)
                .setMessage(message)
                .setPositiveButton("확인", null)
                .show();
    }

    /** The sentence after `err|`, or `fallback` when the native call gave none. */
    private static String reason(String r, String fallback) {
        return r != null && r.startsWith("err|") && r.length() > 4 ? r.substring(4) : fallback;
    }

    private static long parseLong(String s) {
        try {
            return Long.parseLong(s.trim());
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    @SuppressWarnings("deprecation")
    private static int appVersion(MainActivity a) {
        try {
            PackageInfo p = a.getPackageManager().getPackageInfo(a.getPackageName(), 0);
            return Build.VERSION.SDK_INT >= Build.VERSION_CODES.P ? (int) p.getLongVersionCode() : p.versionCode;
        } catch (Throwable t) {
            return 0;   // unknown: the native side then skips the newer-app check
        }
    }

    private static File newestFile(File dir) {
        File[] files = dir.listFiles();
        File best = null;
        if (files != null) {
            for (File f : files) {
                if (f.isFile() && (best == null || f.lastModified() > best.lastModified())) {
                    best = f;
                }
            }
        }
        return best;
    }

    /** Copy everything, refusing past `max` bytes. */
    private static void copy(InputStream in, OutputStream out, long max) throws IOException {
        byte[] buf = new byte[16 * 1024];
        long total = 0;
        int n;
        while ((n = in.read(buf)) != -1) {
            total += n;
            if (total > max) {
                throw new IOException("over " + max + " bytes");
            }
            out.write(buf, 0, n);
        }
    }

    /** The system pickers' answers for 세이브 옮기기. */
    @Override
    @SuppressWarnings("deprecation")
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (resultCode != RESULT_OK || data == null || data.getData() == null) {
            return;
        }
        if (requestCode == REQ_TRANSFER_EXPORT_FILE) {
            finishExportFile(this, data.getData());
        } else if (requestCode == REQ_TRANSFER_IMPORT_PICK) {
            importPicked(this, data.getData());
        }
    }

    /** The vault's restore points, newest first in each slot; the pick is scheduled natively. */
    private static void showSaveRestore(final MainActivity a) {
        final String dir = a.getFilesDir().getAbsolutePath();
        String raw;
        try {
            raw = nativeRestoreList(dir);
        } catch (Throwable t) {
            raw = null;
        }
        if (raw == null || raw.trim().isEmpty()) {
            new AlertDialog.Builder(a)
                    .setTitle("백업에서 복원")
                    .setMessage("보관된 세이브 백업이 없습니다.\n\n백업은 게임이 세이브를 덮어쓸 때마다 자동으로 쌓입니다.")
                    .setPositiveButton("닫기", null)
                    .show();
            return;
        }
        final String[] lines = raw.trim().split("\n");
        final String[] labels = new String[lines.length];
        final SimpleDateFormat fmt = new SimpleDateFormat("MM-dd HH:mm:ss", Locale.KOREA);
        for (int i = 0; i < lines.length; i++) {
            // `slot|ms|bytes|gone` — saves::offer_lines. The line itself is the key.
            String[] f = lines[i].split("\\|");
            long ms = 0;
            try {
                ms = f.length > 1 ? Long.parseLong(f[1]) : 0;
            } catch (NumberFormatException ignored) {
            }
            labels[i] = "슬롯 " + (f.length > 0 ? f[0] : "?") + " · " + fmt.format(new Date(ms))
                    + " · " + (f.length > 2 ? f[2] : "?") + "B"
                    + (f.length > 3 && f[3].equals("1") ? " · 지금 없음" : "");
        }
        new AlertDialog.Builder(a)
                .setTitle("복원할 백업 선택")
                .setItems(labels, (d, which) -> {
                    boolean ok;
                    try {
                        ok = nativeRestoreRequest(dir, lines[which]);
                    } catch (Throwable t) {
                        ok = false;
                    }
                    if (!ok) {
                        toast(a, "복원 예약 실패 — 그 백업은 이제 없습니다");
                        return;
                    }
                    new AlertDialog.Builder(a)
                            .setTitle("복원 예약됨")
                            .setMessage(labels[which]
                                    + "\n\n앱을 다시 실행하면 이 백업으로 돌아갑니다. 지금 슬롯의 세이브는 금고에 보관됩니다."
                                    + "\n지금 닫을까요?")
                            .setPositiveButton("닫기", (d2, w2) -> quitApp())
                            .setNegativeButton("나중에", null)
                            .show();
                })
                .setNegativeButton("취소", null)
                .show();
    }

    /** Every save and the whole vault, copied to the app's external folder (a PC reaches it). */
    private static void showSaveExport(final MainActivity a) {
        File ext = a.getExternalFilesDir(null);
        String said;
        try {
            said = nativeSaveExport(a.getFilesDir().getAbsolutePath(), ext == null ? null : ext.getAbsolutePath());
        } catch (Throwable t) {
            said = null;
        }
        new AlertDialog.Builder(a)
                .setTitle("세이브 폴더로 내보내기")
                .setMessage(said == null ? "내보내지 못했습니다." : said)
                .setPositiveButton("닫기", null)
                .show();
    }

    private static void toast(MainActivity a, String text) {
        Toast.makeText(a, text, Toast.LENGTH_SHORT).show();
    }

    /**
     * From native code: the game's own 「종료」 (MIDlet.notifyDestroyed) — and the restore's 「닫기」
     * above. Every save write is synchronous, fsynced and replaces its file whole, so ending the
     * process loses nothing.
     */
    public static void quitApp() {
        final MainActivity a = instance;
        if (a == null) {
            return;
        }
        a.runOnUiThread(() -> {
            a.finishAndRemoveTask();
            System.exit(0);
        });
    }
}
