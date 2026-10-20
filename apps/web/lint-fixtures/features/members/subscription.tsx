// expect: no-restricted-imports
import { useEffect } from 'react';
export function Bad() {
  useEffect(() => {
    const on = () => {};
    window.addEventListener('resize', on);
    return () => window.removeEventListener('resize', on);
  }, []);
  return null;
}
