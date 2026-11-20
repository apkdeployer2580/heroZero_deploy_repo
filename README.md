# heroZeroNativeAPP — 영웅서기 Zero 안드로이드·PC 리메이크

피처폰 J2ME RPG **영웅서기 Zero 진홍의 사도**(EA Mobile 이식판, 한글 텍스트, 240×320)를 dlunch/wie 에뮬레이터
포크 위에서 **안드로이드 앱**과 **Windows 프로그램**으로 돌리는 프로젝트. 둘은 같은 코드의 두 빌드다.
게임 jar 는 한 바이트도 고치지 않는다 — 클래스가 JVM 에 올라갈 때 바이트를 고치는 게임 패치(`HZ-nnn`)와
앱이 얹은 HUD·기능으로 바꾼다. 같은 구조의 영웅서기2 리메이크(hero2NativeAPP)에서 시작했다.

* 원본 게임과 달라진 것 전부: [`HEROZERO_CHANGES_FROM_ORIGINAL.md`](HEROZERO_CHANGES_FROM_ORIGINAL.md)
* 설치·업데이트·세이브 옮기기: [`SIDELOAD.md`](SIDELOAD.md)
* 설계·근거: [`HEROZERO_REMAKE_PLAN.md`](HEROZERO_REMAKE_PLAN.md) · 출처: [`UPSTREAM.txt`](UPSTREAM.txt) · 라이선스: [`LICENSE`](LICENSE)

## 상태 (2026-11-19 — 안드로이드 versionCode 63 · PC 63)

배포해 플레이 중이다(안드로이드 8.0+ arm64 · Windows x64 — 갤럭시 S9·S25 Ultra, Windows 11 에서 확인).

**앱이 새로 얹은 것**

* hero2·hero3 식 터치 HUD — 장면마다 바뀌는 조작 세트(필드 = 스틱·OK·CLR·퀵 플레이트, 창 = 화살표·OK·CLR),
  배치 편집기, 상태 블록(초상·HP/SP/MP·EXP·버프·포인트 배지), 미니맵, 왼쪽 알림 쌓기, 호스트 문구도 게임 글꼴
* 상단 탭 **SAVE**(묻지 않고 바로 저장)·**MAP**(월드맵 — 장소 24·길 25·배편, 퀘스트 목적지 「!」)
* 게임 창 터치(메뉴·인벤토리·상점·스킬 등의 행·칸을 탭), **대각선 이동**(8 방향), **퀵슬롯 7 칸 × 2 쪽**
* 게임패드(버튼 재배치·위치 배지), 블루투스 키보드, 가로·세로 화면, 노치, 진동
* 세이브 금고(덮기 전 보관)·백업에서 복원·**다른 기기로 보내기/받기**(폰 ↔ PC 같은 파일)
* 소리 — BGM 16 곡(AAC, 고역 복원·루프 이음매)·팡파르·효과음
* **PC 판** — 키보드·마우스·XInput 패드, 창 크기 자유, 그림 밑 퀵 바

**게임 패치 61 건(HZ-001~068)** — 목록은 변경 명세 §2

* 치트상점(SMS 결제) 제거 · 「상점및창고」 처음부터 열림 · 공용창고 → 「계정창고」(메뉴에서 연다)
* 세이브 슬롯 3 → 10 · 가방 28 → 112 칸 · 계정창고 112 칸 · 겹치기 127
* 환경설정 추가 행 — 화면 방향·오토루팅·경험치 배율·드랍률·난이도·몬스터 리젠·조작 방식
* 엔드 콘텐츠 — 가디언 레벨(LV 99 뒤 GL 250)·가디언 장비 16 종·**가디언 모드**(헬 다음 회차)·
  85제 「전설」 장비 102 종·조합서 100 장
* 「부활의 구」·「엘릭서」·일괄판매 · 보스 드랍·반사 패턴·처치 뒤 5 초 대기(드랍을 줍고 스토리)
* 밸런스 — 스탯 공식·스킬 표·SP·스킬 포인트·직업별 조정
* 원본 결함 수정 — 상점 판매·가방이 찬 퀘스트 보상·이야기 깃발 충돌·조준사격 뒤 보스 정지·계정창고 쪽 등

## 준비물

게임 jar(`android_app/app/src/main/assets/herozero.jar`, 원본 바이트 그대로 — md5 `850ad70748b9e1e48b30f6fa60164229`)와
거기서 렌더한 팡파르(`hz_fanfare.wav`), 월드맵 원화(`art/hz_worldmap.png`)가 저장소에 들어 있어 클론한 그대로 빌드된다.
원본 zip 에서 jar 를 다시 꺼내려면 `python -I tools_re/make_hz_asset.py <원본.zip>`(md5 검증).

도구(Windows):

* Android SDK · NDK 25.1.8937393 · JDK 17+
* Rust(rustup) — 기본 `stable-x86_64-pc-windows-gnu` + `cargo-ndk` + 타깃 `aarch64-linux-android`;
  PC 판은 `stable-x86_64-pc-windows-msvc`
* Python 3 + `numpy` · `Pillow` (빌드 때 런처 아이콘·월드맵 그림을 만든다)
* (선택) General MIDI 사운드폰트 — 팡파르(MIDI 1 곡)를 다시 렌더할 때, `HEROZERO_SF3`(없으면 저장소의 WAV)

## 빌드

```powershell
.\build_android.ps1                                   # 디버그 APK -> dist\herozero_0.1.0.<vC>_debug.apk
.\build_android.ps1 -FastDebug -Install -Serial <id>  # 최적화 네이티브 + 설치·실행 (기기가 둘이면 -Serial 필수)
.\build_android.ps1 -Release                          # versionCode +1 + 서명
.\build_desktop.ps1                                   # PC 판 -> dist\herozero_pc_<vC>\ + zip
.\build_desktop.ps1 -Debug -Run                       # PC 개발 빌드를 바로 실행
```

릴리스 서명은 자기 키스토어로 한다 — 환경변수 `HEROZERO_KEYSTORE` · `_KEYSTORE_PASSWORD` · `_KEY_ALIAS` · `_KEY_PASSWORD`,
또는 `%USERPROFILE%\.gradle\gradle.properties` 의 `herozero.*`(`android_app/app/build.gradle.kts` 머리말). 키스토어는 저장소에 두지 않는다.
서명이 다른 APK 는 이미 설치된 앱 위에 업데이트로 설치되지 않는다(`SIDELOAD.md`).
`build_android.ps1` 과 `build_desktop.ps1` 을 동시에 돌리지 말 것 — 두 툴체인이 `Cargo.lock` 을 함께 쓴다.

## 조작

**폰**(가로 기본, 세로는 게임 환경설정 › 화면 방향 — 다음 실행부터)

* 필드: 왼쪽 스틱(조작 방식 = 자유 / 십자키 / 고정) · 오른쪽 아래 OK(공격·대화)와 CLR(메뉴), 그 둘레에 퀵 플레이트 ·
  위 SAVE·MAP 탭 · 톱니(설정·배치 편집·진단 정보·세이브 관리)
* 창: 화살표 넷 · OK · CLR(뒤로). 게임 창의 행·칸을 직접 탭해도 된다 — 첫 탭 = 커서, 다시 탭 = 선택
* 대사: 그림을 탭 = 넘김, 오른쪽 위 ▶▶ = 건너뛰기
* 퀵슬롯 등록: 스킬·정령·물약 창에서 고른 뒤 「퀵슬롯등록」 → 플레이트

**게임패드**: A = OK · B = CLR · X Y LB RB LT RT = 퀵 플레이트 1~6 · R3 = 플레이트 7 · START = 퀵슬롯 쪽 넘김(창에선 메뉴) ·
L3 = SAVE · SELECT = MAP · 스틱·D-pad = 방향. 쪽이 있는 창(상점 구매·가방·계정창고)에선 LB/RB = 이전·다음 쪽.
톱니 › 패드 버튼 설정에서 바꾼다.

**PC**: 화살표 이동(두 개 = 대각선) · X 공격 · Space/Enter 대화·확인 · Esc = CLR · Q W E R A S D = 퀵 플레이트 · Tab = 쪽 ·
숫자 = 메뉴 번호 · F5 저장 · M 월드맵 · F11 전체 화면 · 마우스 왼쪽 = 터치, 오른쪽 = 뒤로, 휠 = 목록.
데이터는 `%APPDATA%\heroZero`, 내보내기·세이브 받기는 「문서」\heroZero.

## 헤드리스 실행 (기기 불필요)

```powershell
.\build_probe.ps1 -Run              # 빌드 + 600 프레임 부팅, 100 프레임마다 PNG
.\target\release\hz_probe.exe --jar android_app\app\src\main\assets\herozero.jar `
    --frames 1200 --ms 20 --keys 300:OK,400:OK --dump 500,1200 --out out\run1 `
    --patches default               # default | none(= 원본 게임) | all | HZ-001,...
python -I tools/hz_watchline.py     # 성능 감시선: 새 게임 → 싸움 → 메뉴 → 저장, 바이트 해시 + 틱
python -I tools/hz_latency.py       # 키 → 그림 지연
```

이 호스트(windows-gnu)에서 맨 `cargo build -p hz_probe` 는 dlltool 때문에 실패한다 — `build_probe.ps1` 이
`tools/dlltool_shim.rs` 를 PATH 에 둔다.

## 시험

```powershell
cargo test -p hz_patches            # 패치 표 (의존성 없음)
cargo test -p hz_java               # 훅·옵션·밸런스·아이템 표
cargo test -p hz_android            # HUD 배치·RMS·금고·세이브 이동 (순수 부분)
cargo +stable-x86_64-pc-windows-msvc test -p hz_android -p hz_desktop --target-dir target_desk   # 앱·워커·PC
python -I tools_re/api_gap.py android_app\app\src\main\assets\herozero.jar                       # 게임이 부르는 API 대 구현
```

## 구성

| 경로 | 내용 |
|---|---|
| `hz_patches/` | 게임 패치 `HZ-nnn` — 원본 클래스 md5 에 묶인 같은 길이 바이트 교체(훅 호출 포함) |
| `hz_java/` | 패치가 부르는 `net/wie/hz/Hooks` 의 구현 — 옵션·밸런스·아이템·퀵슬롯·게임 상태 읽기 |
| `hz_android/` | 앱(`libmain.so`, 폰·PC 공용) — 워커·HUD(`hud`/`touch`/`render`/`placement`)·월드맵·세이브(금고·이동)·소리·패드 |
| `hz_desktop/` | PC 판 `herozero.exe` — 창·키보드·XInput 패드·데이터 폴더 |
| `android_app/` | Gradle GameActivity 프로젝트, applicationId `com.herozeroremake.app` |
| `hz_probe/` | 헤드리스 러너 — 창·소리 없이 가상 시계로 부팅, 키 스크립트, PNG 덤프, 게임 동작 시험 |
| `hz_render/` | 빌드 도구 — 팡파르 MIDI 를 사운드폰트로 WAV 렌더 |
| `vendor/` | wie · rustjava · arm32_cpu · softbuffer · oxisynth 포크 (`UPSTREAM.txt`) |
| `tools/` · `tools_re/` | 성능·지연 측정, 아이템·밸런스 시험, 자산 준비, 맵·드랍표·월드맵 생성, 바이트코드 확인 |

<!-- release-section -->
## 최신 릴리스 (APK)

**[최신 APK 받기 → Releases](https://github.com/apkdeployer2580/heroZero_deploy_repo/releases/latest)**

| 판 | 파일 | versionCode |
|---|---|---|
| 릴리스 | `herozero_0.1.0.64.apk` | 64 |

**이미 설치해서 하던 분 — 세이브를 그대로 이어 하려면**

* 앱을 **지우지 말고** 받은 APK 를 그대로 설치하세요. 업데이트로 덮어 설치되고 세이브가 그대로 남습니다.
* ⚠ 앱을 지우면 세이브도 함께 지워지고 되살릴 수 없습니다.
* 「앱이 설치되지 않았습니다」가 뜨면 서명이 다른 APK(예: 이 소스로 직접 빌드한 APK)가 깔려 있는 것입니다. 그 위에는 공식 APK 를 덮어 설치할 수 없고, 지우면 세이브가 사라집니다.
* 공식 APK 의 서명 인증서 SHA-256: `23abac88934888f56cdacb5297a114b3b9f019ab95005cc20d60869413a1de08`
