// expect: no-restricted-syntax
import * as React from 'react';
export function Bad() {
  React.useLayoutEffect(() => {}, []);
  return null;
}
