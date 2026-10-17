export function readStorage<T>(
  key: string,
  fallback: T,
  valid?: (value: unknown) => value is T,
): T {
  try {
    const raw = localStorage.getItem(`lankide.${key}`);
    if (raw === null) return fallback;
    const value: unknown = JSON.parse(raw);
    return valid && !valid(value) ? fallback : (value as T);
  } catch {
    return fallback;
  }
}
export function writeStorage<T>(key: string, value: T): void {
  try {
    localStorage.setItem(`lankide.${key}`, JSON.stringify(value));
  } catch {
    /* Private browsing and full storage remain usable. */
  }
}
