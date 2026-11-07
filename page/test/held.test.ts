// Random sequences of begins, ends, sets and calls on several parameters, made
// before the snapshot. What goes out after it keeps each parameter's order and
// gestures, its value at each call and at the end, and the order of the calls.
// Where no parameter passes the cap, it is exactly every parameter's pieces in
// the order they were started, which checks the order across parameters. What
// waits stays within the bound `HELD_PER_PARAM` states. Then the cap's choice,
// case by case: which pieces it drops, and which it keeps.

import { expect, test } from "vitest";

import { createClient } from "../src/client";
import { PROTOCOL_VERSION, type PageMessage } from "../src/generated/protocol";
import { HELD_INVOKES, HELD_PER_PARAM, census, heldQueue, type HeldQueue, type ParamWrite } from "../src/held";
import { memoryTransport } from "../src/transport";

const CASES = 4096;
const SEED = 20261106;
const PARAMS = ["a", "b", "c"];
const MAX_OPS = 200;

/** mulberry32: a small seeded generator, uniform on [0, 1). */
function generator(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

type Op = ParamWrite | { type: "invoke"; args: number };

function randomOps(random: () => number): Op[] {
  // Each case draws its parameters, the weight of each kind and the sets in a
  // control's whole drag anew. So long drags, runs of sets or of stray ends,
  // taps with no set, one busy parameter and dense calls all turn up.
  const params = PARAMS.slice(0, 1 + Math.floor(random() * PARAMS.length));
  const [begin, end, set, call, drag] = [random(), random(), random(), random() / 3, random()];
  const dragSets = Math.floor(random() * 4);
  const total = begin + end + set + call + drag;
  const length = Math.floor(random() * MAX_OPS);
  const ops: Op[] = [];
  let sets = 0;
  let calls = 0;
  // Unique values, so each set that goes out names the set it was.
  const setOf = (param: string): Op => ({ type: "set", param, normalized: ++sets / 1024 });
  while (ops.length < length) {
    const param = params[Math.floor(random() * params.length)] ?? "a";
    let pick = random() * total;
    if ((pick -= begin) < 0) ops.push({ type: "gestureBegin", param });
    else if ((pick -= end) < 0) ops.push({ type: "gestureEnd", param });
    else if ((pick -= set) < 0) ops.push(setOf(param));
    else if ((pick -= call) < 0) ops.push({ type: "invoke", args: calls++ });
    else {
      ops.push({ type: "gestureBegin", param });
      for (let i = Math.floor(random() * (dragSets + 1)); i > 0; i--) ops.push(setOf(param));
      ops.push({ type: "gestureEnd", param });
    }
  }
  return ops;
}

interface Walk {
  /** Each set: its value, how many calls came before it, and whether a gesture was open. */
  sets: { value: number; interval: number; inside: boolean }[];
  /** The gesture depth at each call and at the end: as the native side counts
      it, where an end with nothing open closes nothing, and as begins less ends. */
  depths: { native: number; net: number }[];
}

/** One parameter's writes in `stream`, as the native side would see them. */
function walk(stream: readonly (PageMessage | Op)[], param: string): Walk {
  const out: Walk = { sets: [], depths: [] };
  let native = 0;
  let net = 0;
  let interval = 0;
  for (const message of stream) {
    if (message.type === "invoke") {
      out.depths.push({ native, net });
      interval += 1;
    } else if (message.type !== "ready" && message.param === param) {
      if (message.type === "gestureBegin") {
        native += 1;
        net += 1;
      } else if (message.type === "gestureEnd") {
        native = Math.max(0, native - 1);
        net -= 1;
      } else {
        out.sets.push({ value: message.normalized, interval, inside: native > 0 });
      }
    }
  }
  out.depths.push({ native, net });
  return out;
}

const same = (a: unknown, b: unknown): boolean => JSON.stringify(a) === JSON.stringify(b);

/** What `sent` breaks of the properties, against the `ops` that made it. Plain
    checks rather than `expect`, which is too slow for this many cases. */
function problems(ops: Op[], sent: PageMessage[]): string[] {
  const found: string[] = [];
  const calls = (stream: readonly (PageMessage | Op)[]) =>
    stream.flatMap((message) => (message.type === "invoke" ? [message.args] : []));
  if (!same(calls(sent), calls(ops))) found.push("the calls went out out of order, or not all of them");
  for (const param of PARAMS) {
    const made = walk(ops, param);
    const out = walk(sent, param);
    if (!same(out.depths, made.depths)) {
      found.push(`${param}'s gesture depth at the calls and the end: made ${JSON.stringify(made.depths)}, sent ${JSON.stringify(out.depths)}`);
    }
    // Each set that goes out was made, in the same interval and the same
    // gesture state, and they go out in the order they were made.
    let last = -1;
    for (const set of out.sets) {
      const index = made.sets.findIndex((candidate) => candidate.value === set.value);
      if (index <= last) found.push(`${param}'s set ${set.value} was never made, or went out of order`);
      else if (!same(made.sets[index], set)) found.push(`${param}'s set ${set.value} moved: ${JSON.stringify(set)}`);
      last = index;
    }
    // At each call and at the end, the parameter stands at the value the page
    // last gave it before then. A set's interval is the number of calls before it.
    const valueAt = (sets: Walk["sets"], interval: number) =>
      sets.filter((set) => set.interval <= interval).at(-1)?.value;
    for (let interval = 0; interval < made.depths.length; interval++) {
      const want = valueAt(made.sets, interval);
      const got = valueAt(out.sets, interval);
      if (got !== want) found.push(`${param}'s value before call ${interval} was ${got}, made ${want}`);
    }
  }
  return found;
}

/** What `queue` holds beyond the bound `HELD_PER_PARAM` states. */
function overBound(queue: HeldQueue, calls: number): string[] {
  return PARAMS.flatMap((param) => {
    const { whole, pieces, runs } = census(queue, param);
    const found: string[] = [];
    if (whole > Math.max(HELD_PER_PARAM, runs)) found.push(`${param} holds ${whole} whole pieces in ${runs} runs`);
    if (pieces > 2 * whole + 2 * runs) found.push(`${param} holds ${pieces} pieces, ${whole} whole, in ${runs} runs`);
    if (runs > calls + 1) found.push(`${param} has ${runs} runs between ${calls} calls`);
    return found;
  });
}

const withoutIds = (stream: PageMessage[]) =>
  stream.map((message) => (message.type === "invoke" ? { ...message, call: 0 } : message));

/** A piece as the rules at the top of held.ts define it. */
interface ModelPiece {
  param: string;
  kind: "set" | "gesture" | "open" | "ends";
  begins: number;
  value: number | undefined;
  ends: number;
}

/** What goes out for `ops` by those rules: between two calls, every
    parameter's pieces in the order they were started, each as its begins,
    its latest set and its ends. Undefined when a parameter passes the cap,
    whose drops the model leaves to the other checks. */
function inOrderStarted(ops: Op[]): PageMessage[] | undefined {
  const out: PageMessage[] = [];
  let pieces: ModelPiece[] = [];
  const whole = new Map<string, number>();
  const release = () => {
    for (const { param, begins, value, ends } of pieces) {
      for (let i = 0; i < begins; i++) out.push({ type: "gestureBegin", param });
      if (value !== undefined) out.push({ type: "set", param, normalized: value });
      for (let i = 0; i < ends; i++) out.push({ type: "gestureEnd", param });
    }
    pieces = [];
  };
  for (const op of ops) {
    if (op.type === "invoke") {
      release();
      out.push({ type: "invoke", call: 0, command: "c", args: op.args });
      continue;
    }
    const { param } = op;
    let last: ModelPiece | undefined;
    for (const piece of pieces) if (piece.param === param) last = piece;
    const start = (kind: ModelPiece["kind"], piece: Partial<ModelPiece>) =>
      pieces.push({ param, kind, begins: 0, value: undefined, ends: 0, ...piece });
    let madeWhole = false;
    if (last?.kind === "open") {
      if (op.type === "set") last.value = op.normalized;
      else if (op.type === "gestureBegin") last.begins += 1;
      else if (++last.ends === last.begins) {
        last.kind = "gesture";
        madeWhole = true;
      }
    } else if (op.type === "set") {
      if (last?.kind === "set") last.value = op.normalized;
      else {
        start("set", { value: op.normalized });
        madeWhole = true;
      }
    } else if (op.type === "gestureBegin") {
      start("open", { begins: 1 });
    } else if (last?.kind === "ends") {
      last.ends += 1;
    } else {
      start("ends", { ends: 1 });
    }
    if (madeWhole) {
      const count = (whole.get(param) ?? 0) + 1;
      whole.set(param, count);
      if (count > HELD_PER_PARAM) return undefined;
    }
  }
  release();
  return out;
}

test("held writes and calls keep their order, gestures, values at each call and bound", () => {
  const random = generator(SEED);
  const failures: string[] = [];
  let modelled = 0;
  for (let i = 0; i < CASES && failures.length === 0; i++) {
    const seed = Math.floor(random() * 2 ** 32);
    const ops = randomOps(generator(seed));
    const found: string[] = [];
    if (ops.filter((op) => op.type === "invoke").length >= HELD_INVOKES) found.push("too many calls for the case");

    // The queue on its own, to read the bound after every write and call.
    const queue = heldQueue();
    let calls = 0;
    for (const op of ops) {
      if (op.type === "invoke") {
        if (!queue.call({ type: "invoke", call: calls++, command: "c", args: op.args })) found.push("a call was refused");
      } else {
        queue.write(op);
      }
      found.push(...overBound(queue, calls));
    }
    const drained: PageMessage[] = [];
    for (let message = queue.take(); message; message = queue.take()) drained.push(message);

    // The client, which holds the same writes and calls until the snapshot.
    const t = memoryTransport();
    const client = createClient(t.page);
    const bound = new Map(PARAMS.map((param) => [param, client.bindParam(param, () => {})]));
    for (const op of ops) {
      if (op.type === "invoke") {
        client.invoke("c", op.args).catch(() => {});
        continue;
      }
      const param = bound.get(op.param);
      if (op.type === "gestureBegin") param?.gestureBegin();
      else if (op.type === "gestureEnd") param?.gestureEnd();
      else param?.set(op.normalized);
    }
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [] });
    client.dispose();
    const sent = t.native.received.slice(1);

    found.push(...problems(ops, sent));
    if (!same(withoutIds(sent), withoutIds(drained))) found.push("the client sent other than its queue holds");
    const model = inOrderStarted(ops);
    if (model) {
      modelled += 1;
      if (!same(withoutIds(sent), model)) found.push("the pieces went out in another order than they were started");
    }
    if (found.length > 0) failures.push(`seed ${seed}: ${found.slice(0, 3).join("; ")}`);
  }
  expect(failures).toEqual([]);
  // Enough cases stay under the cap for the order across parameters to be
  // checked often.
  expect(modelled).toBeGreaterThan(CASES / 4);
});

/** Everything `queue` holds, oldest first. */
function drain(queue: HeldQueue): PageMessage[] {
  const out: PageMessage[] = [];
  for (let message = queue.take(); message; message = queue.take()) out.push(message);
  return out;
}

const drag = (queue: HeldQueue, param: string, normalized: number): void => {
  queue.write({ type: "gestureBegin", param });
  queue.write({ type: "set", param, normalized });
  queue.write({ type: "gestureEnd", param });
};

const setsOf = (messages: PageMessage[]) => messages.flatMap((m) => (m.type === "set" ? [m.normalized] : []));

test("past the cap the oldest finished gestures go, and the newest ones stay in order", () => {
  const queue = heldQueue();
  const made = HELD_PER_PARAM + 4;
  for (let i = 0; i < made; i++) drag(queue, "a", i);
  const kept = Array.from({ length: HELD_PER_PARAM }, (_, i) => i + made - HELD_PER_PARAM);
  expect(setsOf(drain(queue))).toEqual(kept);
});

test("the value a parameter stood at when a call was made goes out before the call", () => {
  const queue = heldQueue();
  queue.write({ type: "set", param: "a", normalized: 0.3 });
  queue.call({ type: "invoke", call: 0, command: "c", args: null });
  for (let i = 0; i < HELD_PER_PARAM; i++) drag(queue, "a", i);
  const out = drain(queue);
  const call = out.findIndex((m) => m.type === "invoke");
  expect(setsOf(out.slice(0, call))).toEqual([0.3]);
  // Past the cap, the oldest gesture after the call went instead.
  expect(setsOf(out.slice(call + 1))).toEqual(Array.from({ length: HELD_PER_PARAM - 1 }, (_, i) => i + 1));
});

test("every interval keeps its value, even when that leaves more whole pieces than the cap", () => {
  const queue = heldQueue();
  const intervals = HELD_PER_PARAM + 3;
  for (let i = 0; i < intervals; i++) {
    queue.write({ type: "set", param: "a", normalized: i });
    queue.call({ type: "invoke", call: i, command: "c", args: null });
  }
  expect(census(queue, "a").whole).toBe(intervals);
  const out = drain(queue);
  expect(out.map((m) => (m.type === "set" ? m.normalized : "call"))).toEqual(
    Array.from({ length: intervals }, (_, i) => [i, "call"]).flat(),
  );
});

test("a piece of many ends goes out whole, one message at a time", () => {
  const queue = heldQueue();
  const ends = 200_000;
  for (let i = 0; i < ends; i++) queue.write({ type: "gestureEnd", param: "a" });
  expect(census(queue, "a").pieces).toBe(1);
  let count = 0;
  for (let message = queue.take(); message; message = queue.take()) {
    if (message.type === "gestureEnd") count += 1;
  }
  expect(count).toBe(ends);
});
