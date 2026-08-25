import { useEffect, useMemo, useRef, useState } from "react";

import {
  formatClock,
  personLabel,
  type Context,
  type Person,
} from "./api";

/**
 * 처리 직전에 그 회의의 배경 정보를 받는 단계.
 *
 * 전역 설정으로 두면 안 되는 정보다 — 참석자도 안건도 회의마다 다르다. 여기서
 * 받은 내용은 두 곳에 쓰인다.
 *
 *   · 전사: 용어 표기를 고정한다. "틀린표기→정답" 매핑은 그대로 치환된다.
 *   · 요약: 배경 지식이 된다. 특히 참석자를 알려주면 액션 아이템에 담당자가
 *     실제 이름으로 붙는다.
 *
 * 참석자는 조직도에서 고른다. 매번 이름을 다시 치게 하면 결국 아무도 안
 * 적게 되고, 그러면 요약의 담당자가 전부 "미정"이 된다. 조직도에 없는
 * 손님은 옆의 입력란에 직접 적는다.
 *
 * 비워도 진행은 된다. 결과가 나빠질 뿐이라 강제하지 않는다.
 */
export function Brief({
  source,
  seconds,
  initial,
  people,
  onManagePeople,
  onStart,
  onCancel,
}: {
  /** 원본 파일 경로. 이름만 보여준다. */
  source: string;
  /** 녹음이었다면 그 길이. 파일을 넣은 경우엔 아직 모른다. */
  seconds: number | null;
  /** 직전 회의에서 쓴 값. 팀은 대개 반복되므로 미리 채워준다. */
  initial: Context;
  /** 조직도. 참석자를 여기서 고른다. */
  people: Person[];
  /** 조직도 시트를 연다 — 회의 직전에 새 사람을 등록할 수 있어야 한다. */
  onManagePeople: () => void;
  onStart: (context: Context) => void;
  onCancel: () => void;
}) {
  const [context, setContext] = useState<Context>(initial);
  const [showTerms, setShowTerms] = useState(!!initial.terms);
  const first = useRef<HTMLInputElement>(null);

  // 직전 회의의 참석자를 조직도 사람과 그 외 손님으로 가른다. 그래야 지난번에
  // 고른 사람은 다시 골라져 있고, 손님 이름은 입력란에 남는다.
  const initialSplit = useMemo(() => {
    const entries = initial.attendees
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    const matches = (p: Person, entry: string) =>
      personLabel(p) === entry || p.name === entry;
    return {
      picked: new Set(
        people
          .filter((p) => entries.some((entry) => matches(p, entry)))
          .map((p) => p.id),
      ),
      guests: entries
        .filter((entry) => !people.some((p) => matches(p, entry)))
        .join(", "),
    };
    // 처음 그릴 때 한 번이면 된다 — 이후에는 사용자의 선택이 진실이다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const [picked, setPicked] = useState<Set<string>>(initialSplit.picked);
  const [guests, setGuests] = useState(initialSplit.guests);

  useEffect(() => {
    first.current?.focus();
    first.current?.select();
  }, []);

  const set = <K extends keyof Context>(key: K, value: Context[K]) =>
    setContext((c) => ({ ...c, [key]: value }));

  const toggle = (id: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  /** 고른 사람과 직접 적은 손님을 합쳐 최종 참석자 문자열을 만든다. */
  const start = () => {
    const attendees = [
      ...people.filter((p) => picked.has(p.id)).map(personLabel),
      ...guests
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
    ].join(", ");
    onStart({ ...context, attendees });
  };

  // 입력 중 Enter로 바로 시작할 수 있게 — 매번 거치는 화면이라 빨라야 한다.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") start();
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
        {people.length > 0 && (
          <div className="pick-people">
            {people.map((p) => (
              <button
                key={p.id}
                type="button"
                className={`pick-person${picked.has(p.id) ? " on" : ""}`}
                onClick={() => toggle(p.id)}
              >
                {personLabel(p)}
              </button>
            ))}
            <button
              type="button"
              className="pick-person more"
              onClick={onManagePeople}
            >
              ＋ 조직도
            </button>
          </div>
        )}
        <input
          id="brief-who"
          ref={first}
          type="text"
          placeholder={
            people.length > 0
              ? "조직도에 없는 참석자는 직접 적으세요"
              : "김팀장, 박대리, 이과장"
          }
          value={guests}
          onChange={(e) => setGuests(e.target.value)}
        />
        {people.length === 0 && (
          <p className="cast-hint">
            <button type="button" className="link-btn" onClick={onManagePeople}>
              조직도
            </button>
            에 등록해두면 다음 회의부터는 고르기만 하면 됩니다.
          </p>
        )}
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
            placeholder="스마트스토어, 나라장터, 박태리→박대리"
            value={context.terms}
            onChange={(e) => set("terms", e.target.value)}
          />
          <p className="cast-hint">
            화살표로 적으면 잘못 받아적힌 표기를 그대로 바꿔 적습니다. 예:
            박태리→박대리
          </p>
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
        <button className="btn btn--seal" onClick={start}>
          회의록 만들기
        </button>
      </div>
    </div>
  );
}
