// The page's connection to the native side: caches parameter state, sends
// writes as protocol messages, and correlates invoke replies.

import { PROTOCOL_VERSION, type NativeMessage, type ParamState } from "./generated/protocol";
import { heldQueue, type ParamWrite } from "./held";
import { malformed, reasonOf, versionOf } from "./native";
import type { Transport } from "./transport";

export { HELD_INVOKES, HELD_PER_PARAM } from "./held";

/** One parameter as a control sees it. */
export interface BoundParam {
  gestureBegin(): void;
  /** Sets the position, 0 to 1. Rust snaps it to the parameter's range and
      answers with the parameter's state. A position outside 0 to 1 goes out
      limited to it, since the native side refuses it; a non-finite one is dropped.
      Until the snapshot arrives the write waits, and goes out after it; a
      reject drops it. While it waits, the parameter's later sets replace it
      until a begin, an end or a held call comes between. */
  set(normalized: number): void;
  gestureEnd(): void;
  /** The latest state the native side sent, or undefined before the snapshot. */
  current(): ParamState | undefined;
  /** Stops `onState`. Does not close an open gesture. */
  dispose(): void;
}

export interface Client {
  /** Calls `onState` with every state the native side sends for this
      parameter after the call, an unchanged one included: the native side
      answers every `set` and `gestureEnd`, and that answer is news to a
      control still showing its own value even when the store did not move. */
  bindParam(id: string, onState: (state: ParamState) => void): BoundParam;
  /** Runs a native command. Rejects with the native message on `err`, and with
      the failure once the client has failed. Until the snapshot arrives the call
      waits and goes out after it, behind every write made before it; a reject
      fails it with the rest. With `HELD_INVOKES` calls waiting, it rejects at once. */
  invoke(command: string, args: unknown): Promise<unknown>;
  /** Calls `handler` for each native event of that name. Returns its unsubscribe. */
  onEvent(name: string, handler: (payload: unknown) => void): () => void;
  /** Set once the native side speaks another protocol version: it answers a
      `ready` of another version with a `reject` carrying its own version, and
      that fails the client, as does a snapshot of another version from a
      native side that predates `reject`. Set too when a native message is
      not of this protocol's shape, as the native side's own decoder reads it.
      Terminal: the client then drops writes, rejects invokes and ignores
      native messages. */
  failure(): Error | undefined;
  /** Calls `handler` when the client fails (not if it already has). */
  onFailure(handler: (failure: Error) => void): () => void;
  /** Stops listening to the transport and rejects pending invokes. */
  dispose(): void;
}

interface Pending {
  resolve(value: unknown): void;
  reject(reason: Error): void;
}

/** `normalized` limited to 0 to 1, or undefined when it is not finite. */
export function position(normalized: number): number | undefined {
  return Number.isFinite(normalized) ? Math.min(1, Math.max(0, normalized)) : undefined;
}

// A throwing subscriber must not stop delivery to the others, nor the transport
// loop; its error still surfaces as an uncaught one.
function isolate(fn: () => void): void {
  try {
    fn();
  } catch (error) {
    queueMicrotask(() => {
      throw error;
    });
  }
}

function add<K, V>(map: Map<K, Set<V>>, key: K, value: V): () => void {
  let set = map.get(key);
  if (!set) map.set(key, (set = new Set()));
  const members = set;
  members.add(value);
  return () => {
    members.delete(value);
    if (members.size === 0 && map.get(key) === members) map.delete(key);
  };
}

// Call ids are u32 on the wire. The next one wraps past the top, and skips any
// id still waiting for its reply.
const CALL_ID_LIMIT = 0x1_0000_0000;

/** The call id after `last` that is not waiting for a reply. */
export function nextFreeCall(last: number, waiting: (call: number) => boolean): number {
  let call = last;
  do {
    call = (call + 1) % CALL_ID_LIMIT;
  } while (waiting(call));
  return call;
}

function mismatch(native: number, why: string): Error {
  return new Error(`protocol mismatch: native side speaks ${native}, page speaks ${PROTOCOL_VERSION}: ${why}`);
}

// How often `ready` is sent again while the native side has not answered it.
const READY_RETRY_MS = 1000;

/** Subscribes to the transport first, then announces itself, so a snapshot
    sent in answer to `ready` cannot be missed. The native side may not be
    listening yet, so `ready` is sent again every second until a snapshot or a
    reject arrives, the client fails or it is disposed. */
export function createClient(transport: Transport): Client {
  const cache = new Map<string, ParamState>();
  const paramHandlers = new Map<string, Set<(state: ParamState) => void>>();
  const eventHandlers = new Map<string, Set<(payload: unknown) => void>>();
  const failureHandlers = new Set<(failure: Error) => void>();
  const pending = new Map<number, Pending>();
  let nextCall = 0;
  let failed: Error | undefined;
  let disposed = false;
  // True once a snapshot of this protocol version has arrived.
  let ready = false;
  // Before the snapshot answers `ready`, the native side may not be listening,
  // and a page of another protocol version must not reach it at all. So writes
  // and calls wait here and go out, in order, once the snapshot arrives; a
  // reject drops them. `held.ts` says what order and what bound.
  const held = heldQueue();
  // True while `flush` sends. A write or call made then (a state handler can
  // make one, on a transport that delivers replies synchronously) joins what is
  // still held instead of overtaking it.
  let flushing = false;
  let readyTimer: ReturnType<typeof setInterval> | undefined;

  function stopReadyRetry(): void {
    clearInterval(readyTimer);
    readyTimer = undefined;
  }

  // No equality check: a re-sent identical state still reaches the bindings,
  // which deduplicate what they show.
  function store(state: ParamState): void {
    cache.set(state.param, state);
    for (const handler of [...(paramHandlers.get(state.param) ?? [])]) {
      isolate(() => handler(state));
    }
  }

  function rejectAll(reason: Error): void {
    const calls = [...pending.values()];
    pending.clear();
    for (const call of calls) call.reject(reason);
  }

  function fail(reason: Error): void {
    failed = reason;
    stopReadyRetry();
    held.clear();
    rejectAll(reason);
    for (const handler of [...failureHandlers]) isolate(() => handler(reason));
  }

  function onMessage(message: NativeMessage): void {
    if (failed) return;
    // Read as the native side sent it: the transport's type promises a
    // NativeMessage, which only these checks hold it to.
    const received: unknown = message;
    const version = versionOf(received);
    if (version !== undefined && version !== PROTOCOL_VERSION) {
      const why = message.type === "reject" ? (reasonOf(received) ?? "") : "its snapshot is of another version";
      fail(mismatch(version, why));
      return;
    }
    const why = malformed(received);
    if (why !== undefined) {
      fail(new Error(`malformed native message: ${why}`));
      return;
    }
    switch (message.type) {
      case "snapshot":
        stopReadyRetry();
        for (const state of message.params) store(state);
        ready = true;
        flush();
        return;
      case "reject":
        fail(mismatch(message.protocol, message.message));
        return;
      case "param":
        store(message.state);
        return;
      case "reply": {
        const call = pending.get(message.call);
        if (!call) return;
        pending.delete(message.call);
        if (message.result.status === "ok") call.resolve(message.result.value);
        else call.reject(new Error(message.result.message));
        return;
      }
      case "event":
        for (const handler of [...(eventHandlers.get(message.name) ?? [])]) {
          isolate(() => handler(message.payload));
        }
        return;
      default: {
        // A new NativeMessage variant fails the typecheck here until handled.
        const unhandled: never = message;
        void unhandled;
      }
    }
  }

  // Sends what waited for the snapshot, oldest first, until nothing is held. A
  // reject or a dispose inside a send clears what is held, which ends the loop.
  function flush(): void {
    flushing = true;
    try {
      for (let message = held.take(); message; message = held.take()) {
        transport.send(message);
      }
    } finally {
      flushing = false;
    }
  }

  const unlisten = transport.listen(onMessage);
  transport.send({ type: "ready", protocol: PROTOCOL_VERSION });
  if (!ready && !failed) {
    readyTimer = setInterval(
      () => transport.send({ type: "ready", protocol: PROTOCOL_VERSION }),
      READY_RETRY_MS,
    );
  }

  function write(message: ParamWrite): void {
    if (failed || disposed) return;
    if (ready && !flushing) transport.send(message);
    else held.write(message);
  }

  return {
    bindParam(id, onState) {
      const remove = add(paramHandlers, id, onState);
      return {
        gestureBegin: () => write({ type: "gestureBegin", param: id }),
        set: (normalized) => {
          const value = position(normalized);
          if (value !== undefined) write({ type: "set", param: id, normalized: value });
        },
        gestureEnd: () => write({ type: "gestureEnd", param: id }),
        current: () => cache.get(id),
        dispose: remove,
      };
    },
    invoke(command, args) {
      if (failed) return Promise.reject(failed);
      if (disposed) return Promise.reject(new Error("client disposed"));
      const call = nextFreeCall(nextCall, (id) => pending.has(id));
      nextCall = call;
      return new Promise((resolve, reject) => {
        const message = { type: "invoke", call, command, args } as const;
        const now = ready && !flushing;
        if (!now && !held.call(message)) {
          reject(new Error("too many calls are waiting for the native side"));
          return;
        }
        // Before the send, since a transport may deliver the reply inside it.
        pending.set(call, { resolve, reject });
        if (now) transport.send(message);
      });
    },
    onEvent: (name, handler) => add(eventHandlers, name, handler),
    failure: () => failed,
    onFailure: (handler) => {
      failureHandlers.add(handler);
      return () => failureHandlers.delete(handler);
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      held.clear();
      stopReadyRetry();
      unlisten();
      rejectAll(new Error("client disposed"));
    },
  };
}
