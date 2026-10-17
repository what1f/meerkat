// expect: react-hooks/rules-of-hooks
import { useState } from 'react';
export function Bad({ enabled }: { enabled: boolean }) {
  if (enabled) useState(0);
  return null;
}
