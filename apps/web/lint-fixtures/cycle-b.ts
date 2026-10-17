// expect: import-x/no-cycle
import { cycle_a } from './cycle-a';
export function cycle_b(): unknown {
  return cycle_a;
}
