import type * as DTO from '../../lib/api/generated';

// The account this Computer's Built-in Agent is signed in with, if any.
export const builtinAccount = (machine: DTO.MachineCatalogItem | undefined) =>
  machine?.harnesses.find((h) => h.harness_id === 'builtin')?.account ?? null;
