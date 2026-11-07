// What the page client holds before the native side answers its `ready`.
//
// A held call is a barrier: a write made before it goes out before it, and a
// write made after it goes out after it. Between two barriers each parameter
// has a run of pieces, in the order they were made:
//
// - a set, whose value the parameter's later sets replace until a begin or an
//   end;
// - a gesture begun and ended there: its begins, its latest set, its ends;
// - a gesture begun there and still open, which every later write to the
//   parameter between the same barriers joins;
// - ends with no open gesture of the run to join: they close gestures begun
//   before the barrier, or nothing, and go out as the page made them.
//
// Between two barriers the pieces of every parameter go out in the order they
// were started. A write that joins its parameter's newest piece goes out where
// that piece does, possibly before another parameter's earlier write.
//
// A set and an ended gesture are whole: dropping one leaves every other gesture
// open or closed as it was. Only whole pieces are ever dropped, and never the
// newest piece carrying a value between two held calls. So what goes out opens
// and closes each parameter's gestures as the page did, and at each held call
// every parameter stands at the gesture depth and the value the page had given
// it when it made the call.

import type { PageMessage } from "./generated/protocol";

/** A write to one parameter. */
export type ParamWrite = Extract<PageMessage, { type: "gestureBegin" | "set" | "gestureEnd" }>;
/** A call to a native command. */
export type InvokeMessage = Extract<PageMessage, { type: "invoke" }>;

/** The most whole pieces one parameter keeps: sets, and gestures begun and
    ended between the same two held calls. The held calls cut what waits into
    intervals. Past the cap the oldest whole piece goes, except in each interval
    the newest piece carrying a value: it holds what the parameter stood at when
    that interval's call was made. No other piece is ever dropped. So a
    parameter with pieces in `r` intervals keeps at most
    `max(HELD_PER_PARAM, r)` whole pieces, and `r` is at most
    `HELD_INVOKES + 1`. In each interval it has at most one open gesture and one
    piece of ends more than its whole pieces there, so at most
    `2 * max(HELD_PER_PARAM, r) + 2 * r` pieces in all. A piece is a few
    numbers. A gesture or a piece of ends goes out as one message per begin and
    end the page made. */
export const HELD_PER_PARAM = 16;
/** The most calls that wait; the next one is refused. */
export const HELD_INVOKES = 256;

/** A set, whose value the parameter's later sets replace until a begin or an
    end. */
type SetPiece = { kind: "set"; value: number };
/** A gesture begun and ended between two held calls: `begins` begins, the
    latest set, then as many ends. */
type GesturePiece = { kind: "gesture"; begins: number; value: number | undefined };
/** A gesture begun and still open: fewer ends than begins so far. */
type OpenPiece = { kind: "open"; begins: number; ends: number; value: number | undefined };
/** Ends with no open gesture of the run to join. */
type EndsPiece = { kind: "ends"; count: number };
/** `started` orders the pieces of different parameters. */
type Piece = { started: number } & (SetPiece | GesturePiece | OpenPiece | EndsPiece);

const isWhole = (piece: Piece): boolean => piece.kind === "set" || piece.kind === "gesture";

const carriesValue = (piece: Piece): boolean =>
  piece.kind === "set" || ((piece.kind === "gesture" || piece.kind === "open") && piece.value !== undefined);

const startOf = (run: Piece[]): number => run[0]?.started ?? Infinity;

/** The writes between two held calls, and the call that ends them. */
interface Interval {
  /** Each parameter's pieces, oldest first; never an empty one. */
  runs: Map<string, Piece[]>;
  /** Undefined for the newest interval, which takes the writes. */
  call: InvokeMessage | undefined;
}

/** Adds `write` to its parameter's run, a new piece taking `started`. Returns
    whether it made a new whole piece. */
function hold(run: Piece[], write: ParamWrite, started: number): boolean {
  const last = run.at(-1);
  if (last?.kind === "open") {
    if (write.type === "set") last.value = write.normalized;
    else if (write.type === "gestureBegin") last.begins += 1;
    else if (++last.ends === last.begins) {
      run[run.length - 1] = { kind: "gesture", begins: last.begins, value: last.value, started: last.started };
      return true;
    }
    return false;
  }
  switch (write.type) {
    case "set":
      if (last?.kind === "set") {
        last.value = write.normalized;
        return false;
      }
      run.push({ kind: "set", value: write.normalized, started });
      return true;
    case "gestureBegin":
      run.push({ kind: "open", begins: 1, ends: 0, value: undefined, started });
      return false;
    case "gestureEnd":
      if (last?.kind === "ends") last.count += 1;
      else run.push({ kind: "ends", count: 1, started });
      return false;
  }
}

/** A piece being handed out: its begins, then its set if it has one, then its
    ends. `sent` counts what went. Counted rather than listed, since one piece
    can stand for many thousands of begins or ends. */
interface Taking {
  param: string;
  begins: number;
  value: number | undefined;
  ends: number;
  sent: number;
}

function taking(param: string, piece: Piece): Taking {
  const of = (begins: number, value: number | undefined, ends: number): Taking => ({
    param,
    begins,
    value,
    ends,
    sent: 0,
  });
  switch (piece.kind) {
    case "set":
      return of(0, piece.value, 0);
    case "ends":
      return of(0, undefined, piece.count);
    case "gesture":
      return of(piece.begins, piece.value, piece.begins);
    case "open":
      return of(piece.begins, piece.value, piece.ends);
  }
}

/** The next message of the piece being handed out, or undefined once all went. */
function nextOf(out: Taking): ParamWrite | undefined {
  const index = out.sent++;
  const { param, begins, value, ends } = out;
  if (index < begins) return { type: "gestureBegin", param };
  const after = index - begins - (value === undefined ? 0 : 1);
  if (after < 0 && value !== undefined) return { type: "set", param, normalized: value };
  return after < ends ? { type: "gestureEnd", param } : undefined;
}

/** How much one parameter holds, to check the bound `HELD_PER_PARAM` states. */
export interface Census {
  whole: number;
  pieces: number;
  /** The intervals between held calls it has pieces in. */
  runs: number;
}

/** Writes and calls held in order, bounded as `HELD_PER_PARAM` states. */
export interface HeldQueue {
  /** Holds a write. Past the cap it drops a whole piece, as `HELD_PER_PARAM` states. */
  write(write: ParamWrite): void;
  /** Holds a call after every write held so far. Holds nothing and returns
      false when `HELD_INVOKES` calls wait already. */
  call(message: InvokeMessage): boolean;
  /** Takes the oldest held message, or undefined when nothing is held. A write
      held while a piece's messages are being taken goes after them. */
  take(): PageMessage | undefined;
  clear(): void;
}

const censuses = new WeakMap<HeldQueue, (param: string) => Census>();

/** How much `queue` holds for `param`: for tests, which check the bound. */
export function census(queue: HeldQueue, param: string): Census {
  return censuses.get(queue)?.(param) ?? { whole: 0, pieces: 0, runs: 0 };
}

export function heldQueue(): HeldQueue {
  const fresh = (): Interval => ({ runs: new Map(), call: undefined });
  let intervals: Interval[] = [fresh()];
  // The piece `take` is handing out.
  let out: Taking | undefined;
  let started = 0;

  function runsOf(param: string): { interval: Interval; run: Piece[] }[] {
    return intervals.flatMap((interval) => {
      const run = interval.runs.get(param);
      return run ? [{ interval, run }] : [];
    });
  }

  // Each write adds at most one whole piece, so dropping one restores the cap,
  // unless every whole piece is some interval's value.
  function trim(param: string): void {
    const runs = runsOf(param);
    if (runs.flatMap(({ run }) => run).filter(isWhole).length <= HELD_PER_PARAM) return;
    // What the parameter stood at when each interval's call was made.
    const kept = new Set<Piece>();
    for (const { run } of runs) {
      const newest = run.reduce<Piece | undefined>((found, piece) => (carriesValue(piece) ? piece : found), undefined);
      if (newest) kept.add(newest);
    }
    for (const { interval, run } of runs) {
      const index = run.findIndex((piece) => isWhole(piece) && !kept.has(piece));
      if (index < 0) continue;
      run.splice(index, 1);
      // Two pieces of ends the dropped piece kept apart become one, so a run
      // never holds more pieces of ends than whole pieces, plus one.
      const before = run[index - 1];
      const after = run[index];
      if (before?.kind === "ends" && after?.kind === "ends") {
        before.count += after.count;
        run.splice(index, 1);
      }
      if (run.length === 0) interval.runs.delete(param);
      return;
    }
  }

  const queue: HeldQueue = {
    write(write) {
      const { runs } = intervals[intervals.length - 1]!;
      let run = runs.get(write.param);
      if (!run) runs.set(write.param, (run = []));
      if (hold(run, write, started++)) trim(write.param);
    },
    call(message) {
      if (intervals.length - 1 >= HELD_INVOKES) return false;
      intervals[intervals.length - 1]!.call = message;
      intervals.push(fresh());
      return true;
    },
    take() {
      for (;;) {
        const message = out && nextOf(out);
        if (message) return message;
        out = undefined;
        const oldest = intervals[0]!;
        // The parameter whose next piece started first.
        let next: [string, Piece[]] | undefined;
        for (const entry of oldest.runs) {
          if (!next || startOf(entry[1]) < startOf(next[1])) next = entry;
        }
        if (next) {
          const [param, run] = next;
          const piece = run.shift();
          if (run.length === 0) oldest.runs.delete(param);
          if (piece) out = taking(param, piece);
          continue;
        }
        if (!oldest.call) return undefined;
        intervals.shift();
        return oldest.call;
      }
    },
    clear() {
      intervals = [fresh()];
      out = undefined;
    },
  };
  censuses.set(queue, (param) => {
    const runs = runsOf(param);
    const pieces = runs.flatMap(({ run }) => run);
    return { whole: pieces.filter(isWhole).length, pieces: pieces.length, runs: runs.length };
  });
  return queue;
}
