// Lenient decoding helpers. The daemon's JSON grows with each milestone, so
// unknown fields are ignored, states stay open strings, and an optional
// field of the wrong type falls back to its default rather than failing.
// Only an object without its identity (an id, say) is refused.

export class DecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DecodeError";
  }
}

export type Json = Record<string, unknown>;

export function asObject(value: unknown, what: string): Json {
  if (typeof value === "object" && value !== null && !Array.isArray(value)) return value as Json;
  throw new DecodeError(`expected ${what} to be an object`);
}

export function isObject(value: unknown): value is Json {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** A required number. Numeric strings count, since ids drift between the two. */
export function reqNum(o: Json, key: string, what: string): number {
  const value = optNum(o[key]);
  if (value === null) throw new DecodeError(`${what} has no ${key}`);
  return value;
}

export function reqStr(o: Json, key: string, what: string): string {
  const value = o[key];
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  throw new DecodeError(`${what} has no ${key}`);
}

export function optNum(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim() !== "" && Number.isFinite(Number(value))) {
    return Number(value);
  }
  return null;
}

export function num(value: unknown, fallback: number): number {
  return optNum(value) ?? fallback;
}

export function optStr(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

export function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

export function bool(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

export function strings(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

export function numbers(value: unknown): number[] {
  if (!Array.isArray(value)) return [];
  return value.map(optNum).filter((item): item is number => item !== null);
}

/**
 * A list of objects, dropping the ones that don't decode, so one odd row
 * never hides the rest.
 */
export function list<T>(value: unknown, decode: (item: unknown) => T): T[] {
  if (!Array.isArray(value)) return [];
  const out: T[] = [];
  for (const item of value) {
    try {
      out.push(decode(item));
    } catch (error) {
      if (!(error instanceof DecodeError)) throw error;
    }
  }
  return out;
}

/** `{key: [...]}`, the daemon's list pages. */
export function page<T>(body: unknown, key: string, decode: (item: unknown) => T): T[] {
  if (!isObject(body)) throw new DecodeError(`expected an object with ${key}`);
  return list(body[key], decode);
}

/** An RFC 3339 time as Unix milliseconds; 0 if missing or unreadable. */
export function time(value: unknown): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value !== "string") return 0;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? ms : 0;
}
