import type { FullConfig } from '@playwright/test';
import fs from 'fs';

const DEST = 'rowel/tests';

export default async function globalTeardown(config: FullConfig) {
  // Delete previously created temporary directory. Comment for re-running tests manually inside the extension.
  fs.rmSync(DEST, { recursive: true });
}
