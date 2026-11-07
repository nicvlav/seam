import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, test } from "vitest";

import { createClient } from "../src/client";
import { PROTOCOL_VERSION } from "../src/generated/protocol";
import { createHooks } from "../src/hooks";
import { memoryTransport } from "../src/transport";

interface Order {
  id: number;
  total: number;
}
interface Commands {
  getOrder: { args: number; returns: Order };
  ping: { args: void; returns: null };
}

function setup() {
  const t = memoryTransport();
  const { useCommand } = createHooks<Record<string, never>, Record<string, never>, Commands>(createClient(t.page));
  // The native side is listening already, so writes and calls are sent at once.
  t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [] });
  const invokes = () => t.native.received.flatMap((m) => (m.type === "invoke" ? [m] : []));
  const reply = (call: number, result: { status: "ok"; value: unknown } | { status: "err"; message: string }) =>
    act(async () => t.native.send({ type: "reply", call, result }));
  return { useCommand, invokes, reply };
}

describe("useCommand", () => {
  test("tracks pending and data across a resolved run", async () => {
    const { useCommand, invokes, reply } = setup();
    const { result } = renderHook(() => useCommand("getOrder"));
    expect(result.current.pending).toBe(false);
    expect(result.current.data).toBeUndefined();

    let promise!: Promise<Order>;
    act(() => {
      promise = result.current.run(7);
    });
    expect(result.current.pending).toBe(true);
    expect(invokes()[0]).toMatchObject({ command: "getOrder", args: 7 });

    await reply(invokes()[0]!.call, { status: "ok", value: { id: 7, total: 42 } });
    await expect(promise).resolves.toEqual({ id: 7, total: 42 });
    await waitFor(() => expect(result.current.pending).toBe(false));
    expect(result.current.data).toEqual({ id: 7, total: 42 });
    expect(result.current.error).toBeUndefined();
  });

  test("a zero-argument command is run with no arguments, and no result resolves null", async () => {
    const { useCommand, invokes, reply } = setup();
    const { result } = renderHook(() => useCommand("ping"));
    let promise!: Promise<null>;
    act(() => {
      promise = result.current.run();
    });
    expect(invokes()[0]).toMatchObject({ command: "ping", args: undefined });
    await reply(invokes()[0]!.call, { status: "ok", value: null });
    await expect(promise).resolves.toBeNull();
    await waitFor(() => expect(result.current.data).toBeNull());
  });

  test("a rejected call sets error and rejects run", async () => {
    const { useCommand, invokes, reply } = setup();
    const { result } = renderHook(() => useCommand("getOrder"));
    let promise!: Promise<Order>;
    act(() => {
      promise = result.current.run(404);
    });
    const settled = expect(promise).rejects.toThrow("order 404 not found");
    await reply(invokes()[0]!.call, { status: "err", message: "order 404 not found" });
    await settled;
    await waitFor(() => expect(result.current.error?.message).toBe("order 404 not found"));
    expect(result.current.pending).toBe(false);
  });

  test("a later success clears the error", async () => {
    const { useCommand, invokes, reply } = setup();
    const { result } = renderHook(() => useCommand("getOrder"));
    let first!: Promise<Order>;
    act(() => {
      first = result.current.run(1);
    });
    const firstSettled = first.catch(() => undefined);
    await reply(invokes()[0]!.call, { status: "err", message: "bad" });
    await firstSettled;
    await waitFor(() => expect(result.current.error).toBeDefined());

    let second!: Promise<Order>;
    act(() => {
      second = result.current.run(2);
    });
    await reply(invokes()[1]!.call, { status: "ok", value: { id: 2, total: 1 } });
    await second;
    await waitFor(() => expect(result.current.error).toBeUndefined());
  });

  test("pending holds until the last overlapping call settles", async () => {
    const { useCommand, invokes, reply } = setup();
    const { result } = renderHook(() => useCommand("getOrder"));
    let a!: Promise<Order>;
    let b!: Promise<Order>;
    act(() => {
      a = result.current.run(1);
      b = result.current.run(2);
    });
    await reply(invokes()[0]!.call, { status: "ok", value: { id: 1, total: 0 } });
    await a;
    expect(result.current.pending).toBe(true);
    await reply(invokes()[1]!.call, { status: "ok", value: { id: 2, total: 0 } });
    await b;
    await waitFor(() => expect(result.current.pending).toBe(false));
  });

  test("a change of name starts clean, and a late reply to the old name stays out", async () => {
    const { useCommand, invokes, reply } = setup();
    const { result, rerender } = renderHook(({ name }: { name: "getOrder" | "ping" }) => useCommand(name), {
      initialProps: { name: "getOrder" },
    });
    let first!: Promise<unknown>;
    act(() => {
      first = (result.current as { run: (n: number) => Promise<unknown> }).run(7);
    });
    await reply(invokes()[0]!.call, { status: "ok", value: { id: 7, total: 42 } });
    await first;
    await waitFor(() => expect(result.current.data).toEqual({ id: 7, total: 42 }));

    rerender({ name: "ping" });
    expect(result.current.data).toBeUndefined();
    expect(result.current.error).toBeUndefined();
    expect(result.current.pending).toBe(false);

    // A call of the old name settling after the change does not show up.
    let late!: Promise<unknown>;
    rerender({ name: "getOrder" });
    act(() => {
      late = (result.current as { run: (n: number) => Promise<unknown> }).run(8);
    });
    rerender({ name: "ping" });
    expect(result.current.pending).toBe(false);
    await reply(invokes()[1]!.call, { status: "ok", value: { id: 8, total: 1 } });
    await late;
    expect(result.current.data).toBeUndefined();
  });
});
