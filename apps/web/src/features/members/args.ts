// Launch arguments are edited as one line. Quotes keep spaces inside an argument;
// a backslash keeps the next space, quote or backslash literal.
const special = /[\s"'\\]/;

export function formatArgs(args: string[]) {
  return args
    .map((arg) => (arg === '' || special.test(arg) ? `"${arg.replace(/["\\]/g, '\\$&')}"` : arg))
    .join(' ');
}

export function parseArgs(text: string) {
  const args: string[] = [];
  let current = '';
  let started = false;
  let quote: '"' | "'" | null = null;
  for (let i = 0; i < text.length; i++) {
    const char = text[i]!;
    const next = text[i + 1];
    if (quote) {
      if (char === quote) quote = null;
      else if (char === '\\' && quote === '"' && (next === '"' || next === '\\'))
        current += text[++i];
      else current += char;
    } else if (char === '"' || char === "'") {
      quote = char;
      started = true;
    } else if (char === '\\' && next !== undefined && special.test(next)) {
      current += text[++i];
      started = true;
    } else if (/\s/.test(char)) {
      if (started) args.push(current);
      current = '';
      started = false;
    } else {
      current += char;
      started = true;
    }
  }
  if (started) args.push(current);
  return args;
}
