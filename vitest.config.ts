import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    include: ['tests/**/*.test.ts'],
    // A full game materializes tens of thousands of snapshots and can use
    // hundreds of MB while its assertions run. Running test files in parallel
    // multiplies that peak and can OOM the workstation before GC catches up.
    // Keep isolation between files, but execute one isolated file at a time in
    // one worker so the peak is bounded without changing test semantics.
    pool: 'threads',
    fileParallelism: false,
    maxWorkers: 1,
    minWorkers: 1,
  },
});
