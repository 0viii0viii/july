import { useEffect, useRef, useState } from "react";

import { formatClock, type Context } from "./api";

/**
 * 처리 직전에 그 회의의 배경 정보를 받는 단계.
 *
 * 전역 설정으로 두면 안 되는 정보다 — 참석자도 안건도 회의마다 다르다. 여기서
 * 받은 내용은 두 곳에 쓰인다.
 *
 *   · 전사: 이름·용어 표기를 고정한다 ("박대리"가 "박태리"로 나가는 걸 막는다)
 *   · 요약: 배경 지식이 된다. 특히 참석자를 알려주면 액션 아이템에 담당자가
 *     실제 이름으로 붙는다.
 *
 * 비워도 진행은 된다. 결과가 나빠질 뿐이라 강제하지 않는다.
 */
export function Brief({
  source,
  seconds,
  initial,
  onStart,
  onCancel,
}: {
  /** 원본 파일 경로. 이름만 보여준다. */
  source: string;
  /** 녹음이었다면 그 길이. 파일을 넣은 경우엔 아직 모른다. */
  seconds: number | null;
  /** 직전 회의에서 쓴 값. 팀은 대개 반복되므로 미리 채워준다. */
  initial: Context;
  onStart: (context: Context) => void;
  onCancel: () => void;
}) {
  const [context, setContext] = useState<Context>(initial);
  const [showTerms, setShowTerms] = useState(!!initial.terms);
  const first = useRef<HTMLInputElement>(null);

  useEffect(() => {
    first.current?.focus();
    first.current?.select();
  }, []);

  const set = <K extends keyof Context>(key: K, value: Context[K]) =>
    setContext((c) => ({ ...c, [key]: value }));

  // 입력 중 Enter로 바로 시작할 수 있게 — 매번 거치는 화면이라 빨라야 한다.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") onStart(context);
    if (e.key === "Escape") onCancel();
  };

  const fileName = source.split("/").pop() ?? source;

  return (
    <div className="brief" onKeyDown={onKeyDown}>
      <div className="brief-tag">녹음 완료</div>
      <h2>이 회의에 대해 알려주세요</h2>
      <p className="brief-lead">
        참석자와 안건을 적어두면 이름 오인식이 줄고, 액션 아이템에 담당자가
        제대로 붙습니다. 비워둬도 진행은 됩니다.
      </p>

      <div className="brief-source">
        <span className="brief-source-name">{fileName}</span>
        {seconds !== null && <span>{formatClock(seconds)}</span>}
      </div>

      <div className="knob">
        <label htmlFor="brief-who">참석자</label>
        <input
          id="brief-who"
          ref={first}
          type="text"
          placeholder="김팀장, 박대리, 이과장"
          value={context.attendees}
          onChange={(e) => set("attendees", e.target.value)}
        />
      </div>

      <div className="knob">
        <label htmlFor="brief-topic">주제 · 안건</label>
        <input
          id="brief-topic"
          type="text"
          placeholder="신제품 출시 일정 점검"
          value={context.topic}
          onChange={(e) => set("topic", e.target.value)}
        />
      </div>

      {showTerms ? (
        <div className="knob">
          <label htmlFor="brief-terms">자주 틀리는 용어</label>
          <input
            id="brief-terms"
            type="text"
            placeholder="피처 플래그, 스마트스토어, 나라장터"
            value={context.terms}
            onChange={(e) => set("terms", e.target.value)}
          />
        </div>
      ) : (
        <button className="brief-more" onClick={() => setShowTerms(true)}>
          ＋ 자주 틀리는 용어 추가
        </button>
      )}

      <div className="brief-actions">
        <button className="btn" onClick={onCancel}>
          취소
        </button>
        <button className="btn btn--seal" onClick={() => onStart(context)}>
          회의록 만들기
        </button>
      </div>
    </div>
  );
}
