// expect: import-x/no-cycle
import { cycle_b } from './cycle-b';
export function cycle_a(): unknown {
  return cycle_b;
}
