// Checked by `tsc` (npm run typecheck), never run. Each @ts-expect-error line
// fails the build if the line below it stops being an error.

import { createClient } from "../src/client";
import { createHooks, type UseCommandResult, type UseParamResult } from "../src/hooks";
import { memoryTransport } from "../src/transport";
import type { Params } from "./generated/params";

interface Events {
  clipWarning: { channel: number; peakDb: number };
}
interface Commands {
  getOrder: { args: number; returns: { id: number; total: number } };
  rename: { args: { name: string }; returns: boolean };
  ping: { args: void; returns: null };
}

const { useParam, useEvent, useCommand } = createHooks<Params, Events, Commands>(
  createClient(memoryTransport().page),
);

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
function assertType<_T extends true>(): void {}

export function typeChecks(): void {
  const gain: UseParamResult = useParam("gain");
  void gain;
  // @ts-expect-error typo'd parameter id
  useParam("gian");
  // @ts-expect-error an event name is not a parameter id
  useParam("clipWarning");

  useEvent("clipWarning", (payload) => assertType<Equal<typeof payload, { channel: number; peakDb: number }>>());
  // @ts-expect-error typo'd event name
  useEvent("clipWarnin", () => {});
  // @ts-expect-error handler expects the wrong payload type
  useEvent("clipWarning", (payload: string) => void payload);

  const order = useCommand("getOrder");
  assertType<Equal<typeof order, UseCommandResult<number, { id: number; total: number }>>>();
  void order.run(1);
  // @ts-expect-error wrong argument type
  void order.run("1");
  // @ts-expect-error missing argument
  void order.run();
  // @ts-expect-error unknown command
  useCommand("getOrdr");

  const ping = useCommand("ping");
  void ping.run();
  assertType<Equal<Awaited<ReturnType<typeof ping.run>>, null>>();
  // @ts-expect-error a command with void args takes none
  void ping.run(1);

  const rename = useCommand("rename");
  void rename.run({ name: "x" });
  // @ts-expect-error wrong args shape
  void rename.run({ title: "x" });
  assertType<Equal<Awaited<ReturnType<typeof rename.run>>, boolean>>();
}

// @ts-expect-error a command map entry must be { args, returns }
createHooks<Params, Events, { bad: string }>(createClient(memoryTransport().page));
