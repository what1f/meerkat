// expect: react-hooks/exhaustive-deps
import { useEffect } from 'react';
export function Bad({ value }: { value: string }) {
  useEffect(() => {
    console.log(value);
  }, []);
  return null;
}
