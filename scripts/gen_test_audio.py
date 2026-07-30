#!/usr/bin/env python3
"""macOS TTS로 한국어 회의 테스트 음성을 만든다.

whisper가 요구하는 16kHz 모노 WAV로 출력한다.

    python3 scripts/gen_test_audio.py

주의: macOS는 `say -v '?'`에 한국어 음성을 9종 보여주지만, 기본 설치된 것은
Yuna 하나뿐이다. 나머지는 시스템 설정 > 손쉬운 사용 > 음성 콘텐츠에서 따로
내려받아야 하고, 받지 않은 상태로 지정하면 오류 없이 0.14초짜리 무음이
나온다 — 조용히 대사가 통째로 사라진다. 그래서 여기서는 Yuna로 통일하고
화자 구분은 말 속도로만 흉내 낸다.

즉 이 픽스처로는 **전사 정확도만** 검증할 수 있다. 화자분리는 실제로 다른
사람이 녹음한 음성이 필요하다.
"""

import shutil
import subprocess
import tempfile
import wave
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "testdata" / "meeting_ko.wav"

VOICE = "Yuna"

# 화자별 말 속도(분당 단어). 억양은 같지만 최소한의 음향적 차이는 생긴다.
RATES = {"김팀장": 170, "박대리": 200, "이과장": 185, "최선임": 178}

# 실제 개발 회의에 가깝게: 영어 기술용어, 숫자·날짜·금액, 되묻기와 정정,
# 안건 전환, 잡담까지 섞었다. 전사기가 실제로 틀리는 지점들을 노린 구성이다.
#
# (화자, 대사)
SCRIPT = [
    ("김팀장", "자, 다들 모이셨죠. 시작하겠습니다. 오늘 안건은 세 개입니다. 첫째 신제품 출시 일정, 둘째 결제 모듈 이슈, 셋째 다음 스프린트 범위입니다."),
    ("김팀장", "먼저 박대리님, 개발 진행 상황부터 공유해 주세요."),
    ("박대리", "네. 백엔드 API는 팔십 퍼센트 정도 완료했습니다. 전체 마흔두 개 엔드포인트 중에 서른네 개가 끝났고요."),
    ("박대리", "프론트엔드는 다음 주 화요일까지 마무리 예정입니다. 다만 결제 모듈 연동에서 이슈가 하나 있습니다."),
    ("김팀장", "어떤 이슈인가요?"),
    ("박대리", "결제사 쪽 테스트 서버가 이번 주 내내 불안정합니다. 통합 테스트를 아예 못 돌리고 있어요. 최소 삼 일 정도는 지연될 것 같습니다."),
    ("이과장", "잠깐만요, 삼 일이면 QA 일정이 통째로 밀리는데요. 지금 QA 기간이 며칠로 잡혀 있죠?"),
    ("박대리", "원래 오 일 잡아뒀는데, 지금 상황이면 이틀밖에 안 남습니다."),
    ("이과장", "그럼 출시일을 미뤄야 하나요? 마케팅 쪽은 이미 십일월 십오일로 공지가 나간 상태인데요."),
    ("최선임", "제가 하나 제안드리면, 결제 모듈만 피처 플래그로 빼고 나머지는 예정대로 배포하는 방법이 있습니다."),
    ("최선임", "결제는 기존 레거시 모듈로 유지하다가, 안정화되면 스위치만 켜는 식으로요."),
    ("김팀장", "그거 롤백 리스크는 없나요?"),
    ("최선임", "플래그 단위라 즉시 되돌릴 수 있습니다. 다만 두 모듈을 동시에 유지해야 해서 코드가 한동안 지저분해집니다."),
    ("김팀장", "괜찮습니다. 출시일 미루는 것보다는 나아요. 일단 출시일은 십일월 십오일 그대로 유지하는 방향으로 갑시다."),
    ("김팀장", "박대리님은 결제사에 오늘 중으로 연락해서 테스트 서버 일정 확답 받아주세요."),
    ("박대리", "네, 오후에 바로 연락드리겠습니다."),
    ("김팀장", "최선임님은 피처 플래그 설계안을 내일 오전까지 문서로 정리해 주시고요."),
    ("최선임", "알겠습니다. 노션에 올리고 슬랙으로 공유하겠습니다."),
    ("김팀장", "이과장님은 만약을 대비해서 마케팅 팀에 리스크 공유해 주세요. 확정은 아니라고 분명히 전달해 주시고요."),
    ("이과장", "네. 내일 오전까지 정리해서 공유하겠습니다. 그리고 하나 더 있는데요."),
    ("이과장", "지난주에 인프라 비용이 예산을 넘었습니다. 월 사백오십만원 잡아놨는데 오백이십만원 나왔어요."),
    ("김팀장", "원인이 뭔가요?"),
    ("이과장", "스테이징 서버를 밤에도 계속 띄워둔 게 컸습니다. 자동 종료 스케줄을 걸면 대략 월 육십만원 정도 줄일 수 있습니다."),
    ("김팀장", "그럼 그것도 이번 주에 처리하죠. 담당은 최선임님이 맡아주시겠어요?"),
    ("최선임", "네, 금요일까지 적용하겠습니다."),
    ("김팀장", "좋습니다. 마지막 안건인 다음 스프린트 범위는 시간 관계상 다음으로 미루죠."),
    ("김팀장", "다음 회의는 금요일 오후 두 시입니다. 수고하셨습니다."),
]

SAMPLE_RATE = 16000
GAP_SECONDS = 0.4  # 발화 사이 간격 — 세그먼트 경계를 잡을 여지를 준다

# 이보다 짧게 나오면 합성이 사실상 실패한 것으로 본다 (미설치 음성 등).
MIN_SEGMENT_SECONDS = 0.5


def synth(voice: str, rate: int, text: str, dest: Path) -> None:
    """한 대사를 16kHz 모노 WAV로 합성한다."""
    aiff = dest.with_suffix(".aiff")
    subprocess.run(
        ["say", "-v", voice, "-r", str(rate), "-o", str(aiff), text], check=True
    )
    subprocess.run(
        ["afconvert", "-f", "WAVE", "-d", f"LEI16@{SAMPLE_RATE}", "-c", "1",
         str(aiff), str(dest)],
        check=True,
    )
    aiff.unlink()


def main() -> None:
    for tool in ("say", "afconvert"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} 명령을 찾을 수 없습니다 (macOS 전용 스크립트).")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    silence = b"\x00\x00" * int(SAMPLE_RATE * GAP_SECONDS)

    with tempfile.TemporaryDirectory() as tmp:
        tmpdir = Path(tmp)
        frames: list[bytes] = []

        for i, (speaker, text) in enumerate(SCRIPT):
            seg = tmpdir / f"seg{i:02d}.wav"
            synth(VOICE, RATES[speaker], text, seg)

            with wave.open(str(seg), "rb") as w:
                data = w.readframes(w.getnframes())

            seconds = len(data) / (SAMPLE_RATE * 2)
            if seconds < MIN_SEGMENT_SECONDS:
                # 미설치 음성은 오류 없이 무음을 뱉는다. 여기서 잡지 않으면
                # 대사가 통째로 빠진 픽스처를 정상인 줄 알고 쓰게 된다.
                raise SystemExit(
                    f"'{VOICE}' 음성 합성이 실패했습니다 ({seconds:.2f}초). "
                    "시스템 설정 > 손쉬운 사용 > 음성 콘텐츠에서 해당 음성이 "
                    "설치돼 있는지 확인해 주세요."
                )

            frames.append(data)
            frames.append(silence)
            print(f"  [{i + 1}/{len(SCRIPT)}] {speaker}  {seconds:5.1f}초")

        with wave.open(str(OUT), "wb") as out:
            out.setnchannels(1)
            out.setsampwidth(2)
            out.setframerate(SAMPLE_RATE)
            out.writeframes(b"".join(frames))

    seconds = OUT.stat().st_size / (SAMPLE_RATE * 2)
    print(f"\n생성 완료: {OUT}  ({seconds:.1f}초)")

    # 정답 대본 — 전사 정확도를 눈으로 비교할 때 쓴다
    ref = OUT.with_suffix(".txt")
    ref.write_text(
        "\n".join(f"{s}: {t}" for s, t in SCRIPT) + "\n", encoding="utf-8"
    )
    print(f"정답 대본:   {ref}")


if __name__ == "__main__":
    main()
