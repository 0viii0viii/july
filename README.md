# July

회의 녹음을 회의록으로 바꿔주는 macOS 데스크톱 앱.

**전사는 100% 로컬**에서 돌아간다. 녹음 파일이 기기 밖으로 나가지 않는다.
요약만 로컬(Ollama)과 클라우드(Anthropic API) 중에 고른다 — 어느 쪽이든
**서버를 두지 않기 때문에 구독료가 없다.** API를 쓸 경우 사용자 본인 키로
사용자가 직접 비용을 부담한다(BYOK).

## 구조

```
녹음/파일
  → whisper.cpp 전사        (로컬, 무료, 오프라인)
  → 요약                     (Ollama 로컬 · 또는 본인 Anthropic 키)
  → 회의록
```

| 구성 | 선택 |
|---|---|
| 셸 | Tauri v2 (Rust + React) |
| 전사 | whisper.cpp (`whisper-rs`, Metal 가속) |
| 요약 | Ollama `/api/generate` 또는 Anthropic Messages API |

## 준비

```bash
brew install cmake          # whisper.cpp 빌드에 필요
npm install
```

**모델은 손으로 받지 않는다.** 앱을 처음 켜면 준비 화면이 뜨고 거기서 전부
받는다.

- **음성 인식 모델** — 앱이 HuggingFace에서 직접 내려받는다 (진행률 표시)
- **요약 모델** — 기기 메모리를 보고 맞는 것을 추천하고, Ollama의 `/api/pull`로
  앱 안에서 받는다

Ollama 자체는 사용자가 설치해야 한다 (시스템 설치라 앱이 대신 할 수 없다).
설치하지 않으면 준비 화면이 그렇게 안내하고, 대신 Anthropic API 키를 쓰는
길을 제시한다.

### 요약 모델 카탈로그

`src-tauri/src/catalog.rs`에 있다. **태그와 용량은 registry.ollama.ai
매니페스트로 실제 확인한 값이다** — 추측해서 넣으면 설치 버튼이 404로 죽는다.
(예: `gemma4:12b`/`26b`는 실존하지만 `gemma4:4b`는 없고, `qwen3.6`은 Ollama
라이브러리에 아예 없다.)

추천 규칙은 `메모리 − 10GB`를 모델 예산으로 잡고, 그 안에 들어오는 것 중
**10GB 이하에서 가장 큰 것**을 고른다. 메모리가 남는다고 32B를 권하면 요약
한 건에 몇 분씩 걸리는데, 회의록은 정해진 형식으로 뽑아내는 작업이라 모델을
키운 만큼 좋아지지 않는다. 더 큰 걸 원하면 목록에서 직접 고르면 된다.

## 실행

```bash
npm run tauri dev
```

Vite는 **1520 포트**를 쓴다 (기본값 1420은 다른 프로젝트와 충돌해서 옮겼다).
`vite.config.ts`와 `src-tauri/tauri.conf.json`의 `devUrl`은 항상 같이 맞춰야
한다.

## UI 없이 파이프라인만 확인

```bash
cd src-tauri

# 전사만
cargo run --release --example pipeline -- ../testdata/meeting_ko.wav turbo none

# 전사 + 로컬 요약
cargo run --release --example pipeline -- ../testdata/meeting_ko.wav turbo ollama

# 전사 + API 요약
ANTHROPIC_API_KEY=sk-ant-... \
  cargo run --release --example pipeline -- ../testdata/meeting_ko.wav turbo api
```

환경변수로 조정할 수 있는 것들:

| 변수 | 기본값 | 설명 |
|---|---|---|
| `JULY_HINT` | 없음 | 참석자 이름·전문용어. 오인식을 크게 줄인다 (아래 참고) |
| `JULY_MODELS_DIR` | `../models` | whisper 모델 위치 |
| `JULY_OLLAMA_MODEL` | `qwen3:8b` | 로컬 요약 모델 |
| `JULY_ANTHROPIC_MODEL` | `claude-opus-5` | API 요약 모델 |

## 회의 정보는 회의마다 받는다

녹음이 끝나면 처리 전에 **참석자·주제·용어**를 묻는 화면이 뜬다. 전역 설정이
아니다 — 참석자도 안건도 회의마다 다르기 때문이다. 직전 회의 값이 미리 채워져
있어서 같은 팀이면 그냥 Enter를 치면 된다.

받은 정보는 두 곳에 쓰인다.

**전사** — whisper의 `initial_prompt`로 넘어가 표기를 고정한다. 실측 예:

```
없음:  "박태리님 개발 진행 상황 공유해주세요."
있음:  "박대리님, 개발 진행 상황 공유해 주세요."
```

이름뿐 아니라 띄어쓰기와 문장부호까지 개선된다. 다만 `initial_prompt`는 첫
30초 윈도우를 주로 조건화하므로 **긴 회의에서는 뒤로 갈수록 효과가 옅어진다.**
(148초 픽스처에서 "박대리"가 한 번 다시 틀렸다. 청크별 재주입이 필요하다 —
아직 안 됨.)

**요약** — 프롬프트의 배경 지식이 된다. 참석자 명단이 있으면 액션 아이템의
담당자를 실제 이름으로 붙이고, 없으면 "미정"이 늘어난다.

## 테스트 음성

실제 녹음이 없을 때 macOS TTS로 만든다.

```bash
python3 scripts/gen_test_audio.py    # → testdata/meeting_ko.wav (+ 정답 대본)
```

> **주의.** macOS는 `say -v '?'`에 한국어 음성을 9종 보여주지만 기본 설치된
> 것은 **Yuna 하나뿐**이다. 나머지를 지정하면 오류 없이 0.14초짜리 무음이
> 나와서 대사가 통째로 사라진다. 스크립트는 이 경우를 감지해 실패하도록 해
> 뒀다.
>
> 따라서 이 픽스처로 검증할 수 있는 것은 **전사 정확도뿐이다.** 화자분리는
> 실제로 여러 사람이 녹음한 음성이 필요하다.

## 실측 성능 (M2 Pro, large-v3-turbo)

| 항목 | 결과 |
|---|---|
| 전사 | 55초 음성 → 4.0초 (실시간의 13.7배) |
| 로컬 요약 | qwen3:8b 기준 33초 |

## 배포

`v`로 시작하는 태그를 push하면 GitHub Actions가 macOS·Windows 설치파일을
빌드해 **초안 릴리즈**로 올린다. 확인 후 직접 공개하면 된다.

```bash
npm version patch          # package.json 버전 올리기
# src-tauri/tauri.conf.json 의 version 도 같이 맞춘다
git tag v0.1.1 && git push origin v0.1.1
```

태그 없이 빌드만 확인하려면 Actions 탭에서 workflow_dispatch로 돌린다.

빌드 결과물: macOS 유니버설 `.dmg`(x86_64+arm64), Windows `.exe`(NSIS)와
`.msi`, 그리고 업데이터용 `latest.json`과 `.sig`.

### CI에서 걸렸던 것들

같은 실수를 반복하지 않도록 남긴다.

**whisper.cpp는 macOS 10.15 이상을 요구한다.** `std::filesystem`을 쓰는데
유니버설 빌드의 x86_64 타깃은 기본 배포 타깃이 그보다 낮다. 워크플로우에
`MACOSX_DEPLOYMENT_TARGET: '10.15'`가 없으면 `'path' is unavailable`로
컴파일이 막힌다.

**Apple 서명 시크릿을 빈 값으로 미리 적어두면 안 된다.** 시크릿이 없어도
`env:`에 항목이 있으면 환경변수는 빈 문자열로 존재하고, tauri-action은 그걸
인증서가 있다는 신호로 받아 `security import`를 시도하다 실패한다. 없는 것보다
빈 플레이스홀더가 더 나쁘다. 그래서 주석으로만 남겨뒀다.

**갓 만든 저장소는 첫 워크플로우에서 릴리즈 생성이 실패할 수 있다.** 토큰
로그에는 `Contents: write`가 찍히는데도 `Resource not accessible by
integration`이 났다. 몇 분 뒤 재실행하니 그대로 통과했다 — 권한 전파 지연으로
보인다. 설정을 바꾸기 전에 한 번 더 돌려보는 게 낫다.

### macOS 서명

**Apple 유료 계정 없이 ad-hoc 서명(`"signingIdentity": "-"`)으로 나간다.**

이것으로 해결되는 것과 안 되는 것을 구분해야 한다.

- **해결됨** — Apple Silicon에서 서명이 아예 없는 바이너리는 "손상되었습니다"로
  실행 자체가 막힌다. ad-hoc 서명이 이걸 막아준다.
- **해결 안 됨** — 공증(notarization)이 없으므로 처음 열 때 Gatekeeper 경고는
  그대로 뜬다. 릴리즈 노트에 `xattr -dr com.apple.quarantine` 우회법을 적어둔다.

경고를 없애려면 Apple Developer 연 $99가 필요하다. 계정이 생기면
`release.yml`에 이미 자리를 잡아둔 시크릿(`APPLE_CERTIFICATE`,
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`)만
채우면 되고 워크플로우는 고칠 게 없다.

### 자동 업데이트

`createUpdaterArtifacts: true`로 `.sig`와 `latest.json`을 만들고, 앱은 실행 시
한 번 새 버전을 확인한다. 서명 키는 minisign이며 **개인키는 저장소에 없다** —
`TAURI_SIGNING_PRIVATE_KEY` 시크릿에만 있다. 공개키는
`tauri.conf.json`의 `plugins.updater.pubkey`에 들어 있다.

키를 잃어버리면 기존 사용자에게 업데이트를 내보낼 수 없다. 새 키로 바꾸면
사용자가 앱을 다시 설치해야 한다.

## 입력 형식

symphonia로 직접 디코딩하므로 **샘플레이트와 채널 수를 가리지 않는다.**
44.1kHz 스테레오 m4a를 넣어도 앱이 16kHz 모노로 바꿔서 whisper에 넘긴다.

m4a, mp3, wav, flac, aac, ogg, mp4, webm 등을 연다. **macOS 음성 메모가
m4a로 저장하기 때문에** 이게 없으면 정작 맥에서 녹음한 파일을 못 쓴다.

ffmpeg를 함께 배포하지 않는 이유는 용량(70MB+)과 서명·공증 문제 때문이다.
symphonia는 순수 Rust라 바이너리 하나로 끝난다.

실측: 44.1kHz 스테레오 m4a → 148초 음성을 11.3초에 전사 (실시간의 13.2배).
16kHz 모노 WAV와 결과가 사실상 동일하다.

## 남은 일

- [ ] 화자분리 — `sherpa-onnx` 검토 중. whisper 세그먼트에
      `next_segment_speaker_turn`이 있지만 tdrz 모델에서만 채워진다
- [ ] 긴 오디오를 청크로 나눠 힌트를 반복 주입 (현재는 앞부분에만 효과)
- [ ] 회의록 내보내기 (Markdown/PDF)
- [ ] 액션 아이템 체크 상태 저장 (현재는 화면을 벗어나면 초기화)
- [ ] 코드 서명 및 공증

## 검증 상태

정직하게 적는다.

| 항목 | 상태 |
|---|---|
| 한국어 전사 | **실측 완료** — 55초/148초 픽스처 |
| 입력 형식 | **실측 완료** — wav·m4a(모노/스테레오)·44.1kHz 리샘플링 |
| 로컬 요약(Ollama) | **실측 완료** — qwen3:8b |
| Ollama pull API | **실측 완료** — 정상·실패 응답 모두 확인 |
| 모델 카탈로그 | **실측 완료** — 8종 전부 레지스트리에서 존재·용량 확인 |
| macOS·Windows 빌드 | **실측 완료** — v0.1.2가 양쪽 다 성공 |
| 배포물 | **실측 완료** — dmg를 내려받아 유니버설(x86_64+arm64), ad-hoc 서명, 최소 OS 10.15 확인 |
| 마이크 녹음 | **미검증** — 개발 기기(Mac mini)에 마이크가 없다 |
| API 요약(Anthropic) | **미검증** — 키가 없어 호출해보지 못했다 |
| 앱 화면 전체 흐름 | **미검증** — 창을 띄워 끝까지 눌러본 적이 없다 |
| Windows 실행 | **미검증** — 빌드만 됐을 뿐 돌려본 적이 없다 |

### 마이크가 없는 기기

Mac mini·Mac Studio에는 내장 마이크가 없다. 앱은 이 경우를 감지해 녹음 버튼을
막고 이유를 알려준 뒤 파일 입력을 안내한다.

> cpal 0.18에서 `Device`의 `Display` 구현은 `description()` 실패 시
> `fmt::Error`를 돌려주고, `to_string()`은 이걸 **패닉으로 바꾼다.** 입력
> 장치가 없는 기기에서 앱 전체가 죽었다. 이름은 항상 `description()`으로
> 읽어야 한다.
