import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { position, type BoundParam, type Client } from "./client";

/** One command's shape: the single value sent as `args` and the value returned.
    Use `void` for a command that takes no arguments, and `returns: null` for
    one that returns nothing: the native side always sends a value, so such a
    command resolves `null`. */
export interface CommandSpec {
  args: unknown;
  returns: unknown;
}

/** Every entry of a command map must be a CommandSpec. A mapped type rather
    than `Record<string, CommandSpec>`, because an interface has no index
    signature and would not satisfy the Record. */
export type CommandMap<C> = { [K in keyof C]: CommandSpec };

type RunArgs<A> = [A] extends [void] ? [] : [args: A];

export interface UseParamResult {
  /** Normalized position, 0 to 1. While a drag is open it is the dragged value. */
  value: number;
  /** The native side's text for the parameter ("-6.0 dB"); "" before the snapshot. */
  text: string;
  /** Opens a gesture (pointerdown). */
  onDragStart: () => void;
  /** Sets the position during a gesture, limited to 0 to 1. A non-finite
      position is ignored. The same function as `setValue`: the gesture is
      `onDragStart`'s and `onDragEnd`'s. */
  onDrag: (normalized: number) => void;
  /** Closes the gesture (pointerup, pointercancel). The value becomes what the
      native side sent during the drag, then its answer to the end: the store. */
  onDragEnd: () => void;
  /** A one-shot set outside a drag, and the same function as `onDrag`, so
      during a drag it joins the drag. The native side opens a gesture around
      a set outside one. Shows the given position, limited to 0 to 1, until
      the native side's answer arrives. A non-finite position is ignored. */
  setValue: (normalized: number) => void;
  /** Set once the client has failed (protocol mismatch). Writes are then dropped. */
  failure: Error | undefined;
}

export interface UseCommandResult<Args, Returns> {
  /** Runs the command. Concurrent calls are fine. */
  run: (...args: RunArgs<Args>) => Promise<Returns>;
  /** True while any call is in flight. */
  pending: boolean;
  /** The value of the most recently settled successful call. */
  data: Returns | undefined;
  /** The error of the most recently settled call, cleared by a later success. */
  error: Error | undefined;
}

export interface Hooks<Params, Events, Commands extends CommandMap<Commands>> {
  useParam<K extends Extract<keyof Params, string>>(id: K): UseParamResult;
  useEvent<K extends Extract<keyof Events, string>>(
    name: K,
    handler: (payload: Events[K]) => void,
  ): void;
  useCommand<K extends Extract<keyof Commands, string>>(
    name: K,
  ): UseCommandResult<Commands[K]["args"], Commands[K]["returns"]>;
}

function toError(caught: unknown): Error {
  return caught instanceof Error ? caught : new Error(String(caught));
}

function useFailure(client: Client): Error | undefined {
  const [failure, setFailure] = useState(client.failure());
  useEffect(() => {
    setFailure(client.failure());
    return client.onFailure(setFailure);
  }, [client]);
  return failure;
}

/**
 * Creates `useParam`, `useEvent` and `useCommand` for one page. Call it once at
 * module scope with the page's three maps. The parameter map is generated from
 * the Rust declarations (`seam_bridge::typescript::param_map`); the event and
 * command maps are written by hand until Rust generates them too:
 *
 * ```ts
 * export const { useParam, useEvent, useCommand } =
 *   createHooks<Params, Events, Commands>(createClient(transport));
 * ```
 *
 * The maps are fixed here, so each returned hook has a single type parameter,
 * the id, which the compiler infers from the string literal and checks against
 * a concrete map: a typo'd id is an error. A hook that took the map as its own
 * type parameter would leave it free, and TypeScript would then infer a map
 * containing whatever id was typed, so nothing would ever fail.
 */
export function createHooks<Params, Events, Commands extends CommandMap<Commands>>(
  client: Client,
): Hooks<Params, Events, Commands> {
  function useParam<K extends Extract<keyof Params, string>>(id: K): UseParamResult {
    const [shown, setShown] = useState({ value: 0, text: "" });
    const draggingRef = useRef(false);
    // The newest native state that arrived while a drag held the value.
    const heldRef = useRef<{ normalized: number; text: string } | undefined>(undefined);
    const boundRef = useRef<BoundParam | null>(null);
    const failure = useFailure(client);

    // Layout effect so the cached state is on screen before first paint.
    useLayoutEffect(() => {
      // The client passes on every state, repeats included; returning `prev`
      // for an unchanged one lets React skip the render.
      const bound = client.bindParam(id, (state) => {
        if (draggingRef.current) {
          heldRef.current = state;
          setShown((prev) => (prev.text === state.text ? prev : { ...prev, text: state.text }));
          return;
        }
        setShown((prev) =>
          prev.value === state.normalized && prev.text === state.text
            ? prev
            : { value: state.normalized, text: state.text },
        );
      });
      boundRef.current = bound;
      const cached = bound.current();
      setShown(cached ? { value: cached.normalized, text: cached.text } : { value: 0, text: "" });

      return () => {
        // An open gesture would otherwise leave the host recording automation.
        if (draggingRef.current) {
          draggingRef.current = false;
          bound.gestureEnd();
        }
        heldRef.current = undefined;
        bound.dispose();
        boundRef.current = null;
      };
    }, [id]);

    const onDragStart = useCallback(() => {
      if (draggingRef.current) return;
      draggingRef.current = true;
      heldRef.current = undefined;
      boundRef.current?.gestureBegin();
    }, []);

    // A drag's set and a one-shot set differ only in the gesture around them.
    const set = useCallback((normalized: number) => {
      const value = position(normalized);
      if (value === undefined) return;
      setShown((prev) => ({ ...prev, value }));
      boundRef.current?.set(value);
    }, []);

    // Shows what arrived during the drag, then sends the end. The native side
    // answers the end with the store, and that answer must land last, even
    // when a synchronous transport delivers it inside gestureEnd(). When
    // nothing has arrived yet, the dragged value stays until the answers do,
    // rather than flashing back to the value from before the drag.
    const onDragEnd = useCallback(() => {
      if (!draggingRef.current) return;
      draggingRef.current = false;
      const held = heldRef.current;
      heldRef.current = undefined;
      if (held) setShown({ value: held.normalized, text: held.text });
      boundRef.current?.gestureEnd();
    }, []);

    return {
      value: shown.value,
      text: shown.text,
      onDragStart,
      onDrag: set,
      onDragEnd,
      setValue: set,
      failure,
    };
  }

  function useEvent<K extends Extract<keyof Events, string>>(
    name: K,
    handler: (payload: Events[K]) => void,
  ): void {
    // Read through a ref so a new closure each render does not resubscribe;
    // resubscribing would leave a gap in which an event is dropped. The ref is
    // written in a layout effect, not during render, because a render React
    // discards must not change which handler runs.
    const handlerRef = useRef(handler);
    useLayoutEffect(() => {
      handlerRef.current = handler;
    });
    // A layout effect, so the subscription is live as soon as the tree commits;
    // a passive effect runs later and would miss events sent in between.
    useLayoutEffect(
      () => client.onEvent(name, (payload) => handlerRef.current(payload as Events[K])),
      [name],
    );
  }

  function useCommand<K extends Extract<keyof Commands, string>>(
    name: K,
  ): UseCommandResult<Commands[K]["args"], Commands[K]["returns"]> {
    type Args = Commands[K]["args"];
    type Returns = Commands[K]["returns"];
    // Results and the in-flight count are keyed by the command's name, so a
    // change of name starts clean and a late reply to the old name cannot
    // appear as the new one's result.
    const [settled, setSettled] = useState<{ name: string; data?: Returns; error?: Error }>({ name });
    const [inFlight, setInFlight] = useState<Record<string, number>>({});
    const nameRef = useRef<string>(name);
    useLayoutEffect(() => {
      nameRef.current = name;
    }, [name]);

    const run = useCallback(
      async (...args: RunArgs<Args>): Promise<Returns> => {
        // A counter, not a flag: pending holds until the last overlapping call settles.
        const count = (delta: number) =>
          setInFlight((prev) => ({ ...prev, [name]: (prev[name] ?? 0) + delta }));
        count(1);
        try {
          const result = (await client.invoke(name, args[0])) as Returns;
          if (nameRef.current === name) setSettled({ name, data: result });
          return result;
        } catch (caught) {
          const failed = toError(caught);
          if (nameRef.current === name) {
            setSettled((prev) => ({ name, data: prev.name === name ? prev.data : undefined, error: failed }));
          }
          throw failed;
        } finally {
          count(-1);
        }
      },
      [name],
    );

    const current = settled.name === name ? settled : undefined;
    const pending = (inFlight[name] ?? 0) > 0;
    const data = current?.data;
    const error = current?.error;
    return { run, pending, data, error };
  }

  return { useParam, useEvent, useCommand };
}
