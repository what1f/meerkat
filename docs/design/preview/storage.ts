// Design sessions are disposable and never change the app's stored preferences.
const values = new Map<string, unknown>([
  ["language", "zh"],
  ["theme", "light"],
]);
export function readStorage<T>(
  key: string,
  fallback: T,
  valid?: (v: unknown) => v is T,
): T {
  const value = values.get(key);
  return value === undefined || (valid && !valid(value))
    ? fallback
    : (value as T);
}
export function writeStorage<T>(key: string, value: T) {
  values.set(key, value);
}
