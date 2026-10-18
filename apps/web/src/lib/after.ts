// Runs `fn` once after `ms`. Returns a function that cancels it.
export function after(ms: number, fn: () => void): () => void {
  const timer = setTimeout(fn, ms);
  return () => clearTimeout(timer);
}
