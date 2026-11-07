// The protocol messages the native side's own test (crates/bridge/tests/
// fixtures_messages.rs) decodes and re-encodes, driven through the real
// client: what the page sends is exactly the fixture's, and what the native
// side sends has the effect each message names.

import { describe, expect, test } from "vitest";

import messages from "../../crates/bridge/tests/fixtures/messages.json";
import { createClient } from "../src/client";
import { PROTOCOL_VERSION, type NativeMessage, type PageMessage } from "../src/generated/protocol";
import { memoryTransport } from "../src/transport";

interface Fixture {
  page: { name: string; message: PageMessage }[];
  native: { name: string; message: NativeMessage }[];
  nativeLimits: { name: string; message: NativeMessage }[];
}

const fixture = messages as unknown as Fixture;

describe("the shared message fixture", () => {
  test("the page sends exactly the messages the fixture lists", () => {
    const t = memoryTransport();
    const client = createClient(t.page);
    const bound = client.bindParam("gain", () => {});
    for (const { message } of fixture.page) {
      switch (message.type) {
        case "ready":
          // Sent by createClient, which sent it first.
          break;
        case "gestureBegin":
          bound.gestureBegin();
          break;
        case "set":
          bound.set(message.normalized);
          break;
        case "gestureEnd":
          bound.gestureEnd();
          break;
        case "invoke":
          client.invoke(message.command, message.args).catch(() => {});
          break;
        default: {
          const unhandled: never = message;
          void unhandled;
        }
      }
    }
    // The writes and calls waited for the snapshot; it releases them in order.
    expect(t.native.received).toEqual([{ type: "ready", protocol: PROTOCOL_VERSION }]);
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [] });
    client.dispose();
    // What crosses a transport is JSON text: compare what it would carry.
    expect(JSON.parse(JSON.stringify(t.native.received))).toEqual(fixture.page.map((e) => e.message));
    expect(fixture.page[0]?.message).toEqual({ type: "ready", protocol: PROTOCOL_VERSION });
  });

  test("every native message in the fixture does what it says", async () => {
    const t = memoryTransport();
    const client = createClient(t.page);
    const states: unknown[] = [];
    const events: unknown[] = [];
    client.bindParam("gain", (state) => states.push(state));
    client.onEvent("learned", (payload) => events.push(payload));
    const calls = [1, 2, 3].map((n) => client.invoke(`command${n}`, null).then(
      (value) => ({ ok: value }),
      (error: Error) => ({ err: error.message }),
    ));
    const failures: Error[] = [];
    client.onFailure((failure) => failures.push(failure));

    const seen = new Set<string>();
    for (const { message } of fixture.native) {
      seen.add(message.type);
      t.native.send(JSON.parse(JSON.stringify(message)));
      switch (message.type) {
        case "snapshot":
          expect(client.bindParam("mode", () => {}).current()).toEqual(message.params[1]);
          break;
        case "param":
          expect(states.at(-1)).toEqual(message.state);
          break;
        case "event":
          expect(events.at(-1)).toEqual(message.payload);
          break;
        case "reject":
          expect(failures).toHaveLength(1);
          expect(failures[0]?.message).toContain(`${message.protocol}`);
          expect(failures[0]?.message).toContain(message.message);
          break;
        case "reply":
          break;
        default: {
          const unhandled: never = message;
          void unhandled;
        }
      }
    }
    // The three replies in the fixture settle the three calls, in order.
    expect(await Promise.all(calls)).toEqual([{ ok: { renamed: true } }, { ok: null }, { err: "no such command" }]);
    expect([...seen].sort()).toEqual(["event", "param", "reject", "reply", "snapshot"]);
  });

  test("the widest numbers the native side writes are read as it wrote them", () => {
    // The client is no stricter than the Rust decoder: a u32 reaches its
    // largest value, and a position both of its ends.
    expect(fixture.nativeLimits.length).toBeGreaterThan(0);
    for (const { name, message } of fixture.nativeLimits) {
      const t = memoryTransport();
      const client = createClient(t.page);
      t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [] });
      const states: unknown[] = [];
      client.bindParam("gain", (state) => states.push(state));
      t.native.send(JSON.parse(JSON.stringify(message)));
      switch (message.type) {
        case "snapshot":
          // Of another version: a mismatch that names it, not a malformed message.
          expect(client.failure()?.message, name).toContain(`protocol mismatch: native side speaks ${message.protocol},`);
          break;
        case "param":
          expect(client.failure(), name).toBeUndefined();
          expect(states, name).toEqual([message.state]);
          break;
        default:
          // A reply to a call never made is ignored.
          expect(client.failure(), name).toBeUndefined();
      }
      client.dispose();
    }
  });
});
