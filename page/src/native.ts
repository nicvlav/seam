// What the page reads from the native side. The transport's type says every
// message is a NativeMessage, but nothing checks that at runtime: these checks
// do, so one of another shape fails the client instead of throwing inside it.
// They accept every message the Rust encoder writes, and refuse each one the
// shared fixture lists as refused. They differ from the Rust decoder only on
// forms Rust never writes: an array in place of a struct, which serde reads,
// and what `JSON.parse` erases before they run, such as a whole number written
// `1.0` or a key given twice.

import type { NativeMessage } from "./generated/protocol";

/** Each field a message holds, and the check its value must pass. */
type Fields = Record<string, (value: unknown) => boolean>;

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

const isString = (value: unknown): boolean => typeof value === "string";
const anything = (): boolean => true;
/** A u32, as a call id and a protocol version are on the wire. */
const isU32 = (value: unknown): boolean =>
  typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 0xffff_ffff;
/** A position from 0 to 1, which the native side's type keeps it within. */
const isPosition = (value: unknown): boolean => typeof value === "number" && value >= 0 && value <= 1;

/** Why `value` is not an object of exactly `fields`, or undefined when it is.
    A field the protocol does not name is refused, as the Rust decoder refuses
    it. */
function mismatch(value: unknown, fields: Fields): string | undefined {
  if (!isObject(value)) return "not an object";
  for (const key of Object.keys(value)) {
    if (!Object.hasOwn(fields, key)) return `an unknown field ${JSON.stringify(key)}`;
  }
  for (const [key, check] of Object.entries(fields)) {
    if (!Object.hasOwn(value, key)) return `no ${key}`;
    if (!check(value[key])) return `a ${key} of another shape`;
  }
  return undefined;
}

const fits = (fields: Fields) => (value: unknown): boolean => mismatch(value, fields) === undefined;

const isParamState = fits({ param: isString, normalized: isPosition, text: isString });

const isCallResult = (value: unknown): boolean =>
  fits({ status: (status) => status === "ok", value: anything })(value) ||
  fits({ status: (status) => status === "err", message: isString })(value);

// Keyed by every type, so a new NativeMessage variant fails the typecheck
// until it is listed.
const SHAPES: Record<NativeMessage["type"], Fields> = {
  snapshot: { type: anything, protocol: isU32, params: (params) => Array.isArray(params) && params.every(isParamState) },
  reject: { type: anything, protocol: isU32, message: isString },
  param: { type: anything, state: isParamState },
  reply: { type: anything, call: isU32, result: isCallResult },
  event: { type: anything, name: isString, payload: anything },
};

/** Why `message` is not a native message of this protocol, or undefined when
    it is one. */
export function malformed(message: unknown): string | undefined {
  if (!isObject(message)) return "not an object";
  const { type } = message;
  if (typeof type !== "string" || !Object.hasOwn(SHAPES, type)) return `an unknown type ${JSON.stringify(type)}`;
  const why = mismatch(message, SHAPES[type as NativeMessage["type"]]);
  return why === undefined ? undefined : `a ${type} message with ${why}`;
}

/** The version a snapshot or a reject carries, whatever else it holds, so
    that one of another version reads as a mismatch rather than malformed;
    undefined for any other message, or a version that is no u32. */
export function versionOf(message: unknown): number | undefined {
  if (!isObject(message) || (message.type !== "snapshot" && message.type !== "reject")) return undefined;
  return isU32(message.protocol) ? (message.protocol as number) : undefined;
}

/** A reject's reason, when it carries one as text. */
export function reasonOf(message: unknown): string | undefined {
  return isObject(message) && typeof message.message === "string" ? message.message : undefined;
}
