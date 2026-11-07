import { afterEach, describe, expect, test, vi } from "vitest";

import messages from "../../crates/bridge/tests/fixtures/messages.json";
import { HELD_INVOKES, HELD_PER_PARAM, createClient, nextFreeCall } from "../src/client";
import { PROTOCOL_VERSION, type NativeMessage } from "../src/generated/protocol";
import { memoryTransport } from "../src/transport";

function setup() {
  const t = memoryTransport();
  const client = createClient(t.page);
  return { t, client };
}

// A client whose native side has answered, as when it was listening already.
function answeredSetup() {
  const made = setup();
  made.t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [] });
  return made;
}

const gain = (normalized: number, text = `${normalized}`) => ({ param: "gain", normalized, text });

describe("seed", () => {
  test("sends ready only after the listener is installed", () => {
    const t = memoryTransport();
    let listening = false;
    const listen = t.page.listen;
    t.native.listen(() => {
      // The native side answers inside the ready delivery; the snapshot must land.
      t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [gain(0.5)] });
    });
    const client = createClient({
      send: (m) => {
        expect(listening).toBe(true);
        t.page.send(m);
      },
      listen: (h) => {
        listening = true;
        return listen(h);
      },
    });
    expect(t.native.received).toEqual([{ type: "ready", protocol: PROTOCOL_VERSION }]);
    expect(client.bindParam("gain", () => {}).current()).toEqual(gain(0.5));
  });

  test("caches snapshot and later param messages", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    expect(bound.current()).toBeUndefined();
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [gain(0.2)] });
    expect(bound.current()).toEqual(gain(0.2));
    t.native.send({ type: "param", state: gain(0.7) });
    expect(bound.current()).toEqual(gain(0.7));
  });
});

describe("bindParam", () => {
  test("onState fires for each later change of its id only", () => {
    const { t, client } = setup();
    const seen: number[] = [];
    client.bindParam("gain", (s) => seen.push(s.normalized));
    t.native.send({ type: "param", state: { param: "pan", normalized: 0.1, text: "" } });
    t.native.send({ type: "param", state: gain(0.3) });
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [gain(0.4)] });
    expect(seen).toEqual([0.3, 0.4]);
  });

  test("writes become protocol messages", () => {
    const { t, client } = answeredSetup();
    const bound = client.bindParam("gain", () => {});
    bound.gestureBegin();
    bound.set(0.25);
    bound.gestureEnd();
    expect(t.native.received.slice(1)).toEqual([
      { type: "gestureBegin", param: "gain" },
      { type: "set", param: "gain", normalized: 0.25 },
      { type: "gestureEnd", param: "gain" },
    ]);
  });

  test("a re-sent identical state still reaches its bindings", () => {
    const { t, client } = setup();
    const seen: number[] = [];
    client.bindParam("gain", (s) => seen.push(s.normalized));
    t.native.send({ type: "param", state: gain(0.3) });
    t.native.send({ type: "param", state: gain(0.3) });
    expect(seen).toEqual([0.3, 0.3]);
  });

  test("a non-finite position is not sent", () => {
    const { t, client } = answeredSetup();
    const bound = client.bindParam("gain", () => {});
    bound.set(Number.NaN);
    bound.set(Number.NEGATIVE_INFINITY);
    bound.set(0.5);
    expect(t.native.received.slice(1)).toEqual([{ type: "set", param: "gain", normalized: 0.5 }]);
  });

  test("a position outside 0 to 1 goes out limited to it, since the native side refuses it", () => {
    const { t, client } = answeredSetup();
    const bound = client.bindParam("gain", () => {});
    bound.set(1.25);
    bound.set(-0.5);
    expect(t.native.received.slice(1)).toEqual([
      { type: "set", param: "gain", normalized: 1 },
      { type: "set", param: "gain", normalized: 0 },
    ]);
  });

  test("dispose stops onState", () => {
    const { t, client } = setup();
    const seen: number[] = [];
    const bound = client.bindParam("gain", (s) => seen.push(s.normalized));
    bound.dispose();
    t.native.send({ type: "param", state: gain(0.3) });
    expect(seen).toEqual([]);
  });

  test("a throwing subscriber does not starve the others", async () => {
    const { t, client } = setup();
    const seen: number[] = [];
    const uncaught: unknown[] = [];
    const onError = (e: unknown) => uncaught.push(e);
    // The error is rethrown from a microtask, which Node reports as uncaught.
    type Proc = { on(e: string, h: (e: unknown) => void): void; off(e: string, h: (e: unknown) => void): void };
    const proc = (globalThis as unknown as { process: Proc }).process;
    proc.on("uncaughtException", onError);
    client.bindParam("gain", () => {
      throw new Error("boom");
    });
    client.bindParam("gain", (s) => seen.push(s.normalized));
    t.native.send({ type: "param", state: gain(0.3) });
    await new Promise((r) => setTimeout(r, 0));
    proc.off("uncaughtException", onError);
    expect(seen).toEqual([0.3]);
    expect(uncaught).toHaveLength(1);
  });
});

describe("protocol mismatch", () => {
  test("a reject fails the client with the native side's version and reason", async () => {
    const { t, client } = setup();
    const early = client.invoke("x", null);
    t.native.send({ type: "reject", protocol: PROTOCOL_VERSION + 1, message: "serves only the newer protocol" });
    const message = client.failure()?.message ?? "";
    expect(message).toMatch(/protocol mismatch/);
    expect(message).toContain(`native side speaks ${PROTOCOL_VERSION + 1}`);
    expect(message).toContain("serves only the newer protocol");
    await expect(early).rejects.toThrow(/protocol mismatch/);
    await expect(client.invoke("y", null)).rejects.toThrow(/protocol mismatch/);
    const before = t.native.received.length;
    client.bindParam("gain", () => {}).set(0.1);
    expect(t.native.received).toHaveLength(before);
  });

  test("fails the client, visibly to listeners, bindings and invoke", async () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    const failures: Error[] = [];
    client.onFailure((f) => failures.push(f));
    const early = client.invoke("x", null);

    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION + 1, params: [gain(0.5)] });

    expect(client.failure()?.message).toMatch(/protocol mismatch/);
    expect(failures).toHaveLength(1);
    expect(bound.current()).toBeUndefined();
    await expect(early).rejects.toThrow(/protocol mismatch/);
    await expect(client.invoke("y", null)).rejects.toThrow(/protocol mismatch/);

    const before = t.native.received.length;
    bound.set(0.1);
    expect(t.native.received).toHaveLength(before);
    t.native.send({ type: "param", state: gain(0.9) });
    expect(bound.current()).toBeUndefined();
  });
});

describe("invoke", () => {
  test("resolves with the value of an ok reply", async () => {
    const { t, client } = answeredSetup();
    const p = client.invoke("add", [1, 2]);
    expect(t.native.received[1]).toEqual({ type: "invoke", call: expect.any(Number), command: "add", args: [1, 2] });
    const sent = t.native.received[1];
    if (sent?.type !== "invoke") throw new Error("expected invoke");
    t.native.send({ type: "reply", call: sent.call, result: { status: "ok", value: 3 } });
    await expect(p).resolves.toBe(3);
  });

  test("rejects with the message of an err reply", async () => {
    const { t, client } = answeredSetup();
    const p = client.invoke("add", null);
    const sent = t.native.received[1];
    if (sent?.type !== "invoke") throw new Error("expected invoke");
    t.native.send({ type: "reply", call: sent.call, result: { status: "err", message: "nope" } });
    await expect(p).rejects.toThrow("nope");
    await expect(p).rejects.toBeInstanceOf(Error);
  });

  test("call ids are unique among pending calls and replies route by id", async () => {
    const { t, client } = answeredSetup();
    const a = client.invoke("a", null);
    const b = client.invoke("b", null);
    const calls = t.native.received.flatMap((m) => (m.type === "invoke" ? [m.call] : []));
    expect(new Set(calls).size).toBe(2);
    t.native.send({ type: "reply", call: calls[1]!, result: { status: "ok", value: "B" } });
    t.native.send({ type: "reply", call: calls[0]!, result: { status: "ok", value: "A" } });
    await expect(a).resolves.toBe("A");
    await expect(b).resolves.toBe("B");
  });

  test("ignores a reply for an unknown call, and a repeated reply", async () => {
    const { t, client } = answeredSetup();
    t.native.send({ type: "reply", call: 999, result: { status: "ok", value: 1 } });
    const p = client.invoke("a", null);
    const sent = t.native.received[1];
    if (sent?.type !== "invoke") throw new Error("expected invoke");
    t.native.send({ type: "reply", call: sent.call, result: { status: "ok", value: 1 } });
    t.native.send({ type: "reply", call: sent.call, result: { status: "err", message: "late" } });
    await expect(p).resolves.toBe(1);
  });
});

describe("onEvent", () => {
  test("delivers by name until unsubscribed", () => {
    const { t, client } = setup();
    const seen: unknown[] = [];
    const off = client.onEvent("clip", (p) => seen.push(p));
    t.native.send({ type: "event", name: "other", payload: 0 });
    t.native.send({ type: "event", name: "clip", payload: 1 });
    off();
    t.native.send({ type: "event", name: "clip", payload: 2 });
    expect(seen).toEqual([1]);
  });
});

test("dispose stops listening and rejects pending calls", async () => {
  const { t, client } = setup();
  const p = client.invoke("a", null);
  client.dispose();
  await expect(p).rejects.toThrow(/disposed/);
  const seen: number[] = [];
  client.bindParam("gain", (s) => seen.push(s.normalized));
  t.native.send({ type: "param", state: gain(0.5) });
  expect(seen).toEqual([]);
});

describe("call ids", () => {
  test("count up from the last id", () => {
    expect(nextFreeCall(0, () => false)).toBe(1);
    expect(nextFreeCall(41, () => false)).toBe(42);
  });

  test("wrap to zero after the largest u32", () => {
    expect(nextFreeCall(0xffff_fffe, () => false)).toBe(0xffff_ffff);
    expect(nextFreeCall(0xffff_ffff, () => false)).toBe(0);
  });

  test("skip ids still waiting, across the wrap too", () => {
    const waiting = new Set([5, 6]);
    expect(nextFreeCall(4, (id) => waiting.has(id))).toBe(7);
    const wrapped = new Set([0, 1]);
    expect(nextFreeCall(0xffff_ffff, (id) => wrapped.has(id))).toBe(2);
  });
});

describe("before the snapshot", () => {
  const snapshot = (t: ReturnType<typeof memoryTransport>) =>
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [] });
  const sent = (t: ReturnType<typeof memoryTransport>) => t.native.received.slice(1);

  test("a write waits for the snapshot and goes out after it", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    bound.gestureBegin();
    bound.set(0.25);
    bound.gestureEnd();
    expect(sent(t)).toEqual([]);
    snapshot(t);
    expect(sent(t)).toEqual([
      { type: "gestureBegin", param: "gain" },
      { type: "set", param: "gain", normalized: 0.25 },
      { type: "gestureEnd", param: "gain" },
    ]);
    bound.set(0.5);
    expect(sent(t)).toHaveLength(4);
  });

  test("a call waits, goes out after the snapshot and settles on its reply", async () => {
    const { t, client } = setup();
    const call = client.invoke("add", [1, 2]);
    expect(sent(t)).toEqual([]);
    snapshot(t);
    const out = sent(t)[0];
    if (out?.type !== "invoke") throw new Error("expected invoke");
    expect(out).toEqual({ type: "invoke", call: out.call, command: "add", args: [1, 2] });
    t.native.send({ type: "reply", call: out.call, result: { status: "ok", value: 3 } });
    await expect(call).resolves.toBe(3);
  });

  test("writes and calls go out in the order they were made", () => {
    const { t, client } = setup();
    const gainParam = client.bindParam("gain", () => {});
    const pan = client.bindParam("pan", () => {});
    gainParam.set(0.1);
    void client.invoke("a", null);
    pan.set(0.2);
    void client.invoke("b", null);
    snapshot(t);
    expect(sent(t).map((m) => (m.type === "set" ? `set:${m.param}` : m.type === "invoke" ? `invoke:${m.command}` : m.type))).toEqual([
      "set:gain",
      "invoke:a",
      "set:pan",
      "invoke:b",
    ]);
  });

  test("parameters go out in the order their writes started, a set that joins an earlier one going where it is", () => {
    const { t, client } = setup();
    const gainParam = client.bindParam("gain", () => {});
    const pan = client.bindParam("pan", () => {});
    gainParam.gestureBegin();
    gainParam.set(0.1);
    gainParam.gestureEnd();
    pan.set(0.2);
    gainParam.gestureBegin();
    gainParam.set(0.3);
    gainParam.gestureEnd();
    pan.set(0.4);
    snapshot(t);
    expect(sent(t).map((m) => (m.type === "set" ? `${m.param}:${m.normalized}` : `${m.type}`))).toEqual([
      "gestureBegin",
      "gain:0.1",
      "gestureEnd",
      "pan:0.4",
      "gestureBegin",
      "gain:0.3",
      "gestureEnd",
    ]);
  });

  test("sets of one parameter coalesce to the latest", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    for (let i = 1; i <= 1000; i++) bound.set(i / 1000);
    snapshot(t);
    expect(sent(t)).toEqual([{ type: "set", param: "gain", normalized: 1 }]);
  });

  test("a drag held open keeps its gesture and its latest position", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    bound.gestureBegin();
    for (let i = 1; i <= 100; i++) bound.set(i / 100);
    snapshot(t);
    expect(sent(t)).toEqual([
      { type: "gestureBegin", param: "gain" },
      { type: "set", param: "gain", normalized: 1 },
    ]);
    bound.gestureEnd();
    expect(sent(t).at(-1)).toEqual({ type: "gestureEnd", param: "gain" });
  });

  test("nested gestures on one parameter keep every begin and end", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    bound.gestureBegin();
    bound.gestureBegin();
    bound.set(0.5);
    bound.gestureEnd();
    bound.gestureEnd();
    snapshot(t);
    expect(sent(t).map((m) => m.type)).toEqual(["gestureBegin", "gestureBegin", "set", "gestureEnd", "gestureEnd"]);
  });

  test("what waits is bounded: finished gestures past the limit are dropped oldest first, an open one never", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    for (let i = 0; i < HELD_PER_PARAM * 4; i++) {
      bound.gestureBegin();
      bound.set(i / 1000);
      bound.gestureEnd();
    }
    bound.gestureBegin();
    bound.set(0.9);
    snapshot(t);
    const messages = sent(t);
    // The whole gestures kept, and the open one's begin and set.
    expect(messages.length).toBeLessThanOrEqual(HELD_PER_PARAM * 3 + 2);
    expect(messages.at(-2)).toEqual({ type: "gestureBegin", param: "gain" });
    expect(messages.at(-1)).toEqual({ type: "set", param: "gain", normalized: 0.9 });
    const begins = messages.filter((m) => m.type === "gestureBegin").length;
    const ends = messages.filter((m) => m.type === "gestureEnd").length;
    expect(begins - ends).toBe(1);
  });

  test("a set made after a held call goes out after it, not merged into an earlier set", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    bound.set(0.1);
    void client.invoke("resetAll", null);
    bound.set(0.3);
    snapshot(t);
    expect(sent(t).map((m) => (m.type === "set" ? `set:${m.normalized}` : m.type === "invoke" ? `invoke:${m.command}` : m.type))).toEqual([
      "set:0.1",
      "invoke:resetAll",
      "set:0.3",
    ]);
  });

  test("a gesture end made after a held call goes out after it", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    bound.gestureBegin();
    bound.set(0.5);
    void client.invoke("resetAll", null);
    bound.gestureEnd();
    snapshot(t);
    expect(sent(t).map((m) => (m.type === "invoke" ? `invoke:${m.command}` : m.type))).toEqual([
      "gestureBegin",
      "set",
      "invoke:resetAll",
      "gestureEnd",
    ]);
  });

  test("a write made by a state handler during the flush replaces the held set it follows, so the last write wins", () => {
    const { t, client } = setup();
    const gainParam = client.bindParam("gain", () => {
      pan.set(0.9);
    });
    const pan = client.bindParam("pan", () => {});
    t.native.listen((m) => {
      if (m.type === "set" && m.param === "gain") t.native.send({ type: "param", state: gain(m.normalized) });
    });
    gainParam.set(0.5);
    pan.set(0.1);
    snapshot(t);
    expect(sent(t).map((m) => (m.type === "set" ? `${m.param}:${m.normalized}` : m.type))).toEqual([
      "gain:0.5",
      "pan:0.9",
    ]);
  });

  test("a call made by a state handler during the flush goes out after the held writes", () => {
    const { t, client } = setup();
    const gainParam = client.bindParam("gain", () => {
      void client.invoke("later", null);
    });
    const pan = client.bindParam("pan", () => {});
    t.native.listen((m) => {
      if (m.type === "set" && m.param === "gain") t.native.send({ type: "param", state: gain(m.normalized) });
    });
    gainParam.set(0.5);
    pan.set(0.1);
    snapshot(t);
    expect(sent(t).map((m) => (m.type === "invoke" ? `invoke:${m.command}` : m.type))).toEqual(["set", "set", "invoke:later"]);
  });

  test("the cap never drops the only entry that carries a parameter's value", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    bound.set(0.5);
    for (let i = 0; i < HELD_PER_PARAM; i++) {
      bound.gestureBegin();
      bound.gestureEnd();
    }
    snapshot(t);
    expect(sent(t)).toContainEqual({ type: "set", param: "gain", normalized: 0.5 });
  });

  const countOf = (messages: ReturnType<typeof sent>, type: string) =>
    messages.filter((m) => m.type === type).length;

  test("a gesture a held call splits keeps its end however many writes follow", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    bound.gestureBegin();
    bound.set(0.1);
    void client.invoke("between", null);
    bound.gestureEnd();
    for (let i = 0; i < HELD_PER_PARAM; i++) {
      bound.gestureBegin();
      bound.set(i / 100);
      bound.gestureEnd();
    }
    snapshot(t);
    const messages = sent(t);
    expect(countOf(messages, "gestureBegin")).toBe(countOf(messages, "gestureEnd"));
    const call = messages.findIndex((m) => m.type === "invoke");
    expect(messages.findIndex((m) => m.type === "gestureBegin")).toBeLessThan(call);
    expect(messages.findIndex((m) => m.type === "gestureEnd")).toBeGreaterThan(call);
  });

  test("the pieces of gestures held calls split are all kept", () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    const splits = HELD_PER_PARAM * 2;
    for (let i = 0; i < splits; i++) {
      bound.gestureBegin();
      void client.invoke("between", i);
    }
    for (let i = 0; i < splits; i++) bound.gestureEnd();
    snapshot(t);
    const messages = sent(t);
    expect(countOf(messages, "gestureBegin")).toBe(splits);
    expect(countOf(messages, "gestureEnd")).toBe(splits);
    expect(countOf(messages, "invoke")).toBe(splits);
  });

  test("calls past the limit reject at once, and the ones held still go out", async () => {
    const { t, client } = setup();
    const held = Array.from({ length: HELD_INVOKES }, (_, i) => client.invoke("c", i));
    await expect(client.invoke("c", "one too many")).rejects.toThrow(/too many calls/);
    snapshot(t);
    expect(sent(t)).toHaveLength(HELD_INVOKES);
    client.dispose();
    await Promise.allSettled(held);
  });

  test("a reject drops the held writes and rejects the held calls", async () => {
    const { t, client } = setup();
    const bound = client.bindParam("gain", () => {});
    bound.gestureBegin();
    bound.set(0.3);
    const call = client.invoke("x", null);
    t.native.send({ type: "reject", protocol: PROTOCOL_VERSION + 1, message: "newer" });
    await expect(call).rejects.toThrow(/protocol mismatch/);
    expect(sent(t)).toEqual([]);
  });

  test("a snapshot of another version sends nothing either", async () => {
    const { t, client } = setup();
    client.bindParam("gain", () => {}).set(0.3);
    const call = client.invoke("x", null);
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION + 1, params: [] });
    await expect(call).rejects.toThrow(/protocol mismatch/);
    expect(sent(t)).toEqual([]);
  });

  test("a call disposed while waiting rejects and is never sent", async () => {
    const { t, client } = setup();
    const call = client.invoke("x", null);
    client.dispose();
    await expect(call).rejects.toThrow(/disposed/);
    snapshot(t);
    expect(sent(t)).toEqual([]);
  });
});

describe("ready retry", () => {
  afterEach(() => {
    vi.useRealTimers();
  });
  const readies = (t: ReturnType<typeof memoryTransport>) =>
    t.native.received.filter((m) => m.type === "ready").length;

  test("repeats until a snapshot arrives", () => {
    vi.useFakeTimers();
    const { t } = setup();
    expect(readies(t)).toBe(1);
    vi.advanceTimersByTime(3000);
    expect(readies(t)).toBe(4);
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [] });
    vi.advanceTimersByTime(5000);
    expect(readies(t)).toBe(4);
  });

  test("does not repeat when the first ready is answered at once", () => {
    vi.useFakeTimers();
    const t = memoryTransport();
    t.native.listen(() => t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [] }));
    createClient(t.page);
    vi.advanceTimersByTime(5000);
    expect(readies(t)).toBe(1);
  });

  test("does not repeat when the first ready is rejected at once", () => {
    vi.useFakeTimers();
    const t = memoryTransport();
    t.native.listen(() => t.native.send({ type: "reject", protocol: PROTOCOL_VERSION + 1, message: "no" }));
    createClient(t.page);
    vi.advanceTimersByTime(5000);
    expect(readies(t)).toBe(1);
  });

  test("stops when the client is disposed", () => {
    vi.useFakeTimers();
    const { t, client } = setup();
    vi.advanceTimersByTime(1000);
    client.dispose();
    vi.advanceTimersByTime(5000);
    expect(readies(t)).toBe(2);
  });

  test("stops when a reject fails the client", () => {
    vi.useFakeTimers();
    const { t, client } = setup();
    t.native.send({ type: "reject", protocol: PROTOCOL_VERSION + 1, message: "no" });
    expect(client.failure()).toBeDefined();
    vi.advanceTimersByTime(5000);
    expect(readies(t)).toBe(1);
  });

  test("stops when a snapshot of another version fails the client", () => {
    vi.useFakeTimers();
    const { t, client } = setup();
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION + 1, params: [] });
    expect(client.failure()).toBeDefined();
    vi.advanceTimersByTime(5000);
    expect(readies(t)).toBe(1);
  });
});

describe("a malformed native message", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  // The transport's type holds a well-formed message; a native side may send
  // anything, which the transport delivers as it is.
  const sendRaw = (t: ReturnType<typeof memoryTransport>, message: unknown) =>
    t.native.send(message as NativeMessage);

  test("a snapshot without params fails the client visibly, stops the ready retry and drops what waits", async () => {
    vi.useFakeTimers();
    const { t, client } = setup();
    const failures: Error[] = [];
    client.onFailure((failure) => failures.push(failure));
    client.bindParam("gain", () => {}).set(0.5);
    const call = client.invoke("undo", null);
    sendRaw(t, { type: "snapshot", protocol: PROTOCOL_VERSION });
    expect(client.failure()?.message).toBe("malformed native message: a snapshot message with no params");
    expect(failures).toEqual([client.failure()]);
    await expect(call).rejects.toBe(client.failure());
    vi.advanceTimersByTime(5000);
    expect(t.native.received).toEqual([{ type: "ready", protocol: PROTOCOL_VERSION }]);
  });

  test("one after the snapshot fails the client too, and later messages are ignored", () => {
    const { t, client } = answeredSetup();
    const seen: number[] = [];
    client.bindParam("gain", (state) => seen.push(state.normalized));
    sendRaw(t, { type: "param", state: { param: "gain", normalized: Number.NaN, text: "" } });
    expect(client.failure()?.message).toBe("malformed native message: a param message with a state of another shape");
    t.native.send({ type: "param", state: gain(0.5) });
    expect(seen).toEqual([]);
  });

  test("every native message the Rust decoder refuses fails the client as malformed", () => {
    // Malformed, not a mismatch: a fractional protocol must not read as
    // another version.
    const refused = (messages as unknown as { refusedAsNative: { name: string; text: string }[] }).refusedAsNative;
    expect(refused.length).toBeGreaterThan(0);
    for (const { name, text } of refused) {
      const { t, client } = setup();
      sendRaw(t, JSON.parse(text));
      expect(client.failure()?.message, name).toMatch(/^malformed native message: /);
      client.dispose();
    }
  });

  test("a message of an unknown type, or no object at all, fails the client", () => {
    for (const message of [{ type: "teleport" }, null, "snapshot", [], { protocol: PROTOCOL_VERSION }]) {
      const { t, client } = answeredSetup();
      sendRaw(t, message);
      expect(client.failure()?.message, JSON.stringify(message)).toMatch(/^malformed native message: /);
    }
  });

  test("a reject of another version reads as a mismatch whatever else it holds", () => {
    const { t, client } = setup();
    sendRaw(t, { type: "reject", protocol: PROTOCOL_VERSION + 1, message: "too new", extra: true });
    expect(client.failure()?.message).toContain("protocol mismatch");
    expect(client.failure()?.message).toContain("too new");
  });
});
