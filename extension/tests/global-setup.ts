import type { FullConfig } from '@playwright/test';
import fs from 'fs';

const DEST = 'rowel/tests';

export default async function globalSetup(config: FullConfig) {
  // Create a temporary directory and copy tests/* to rowel/tests
  fs.existsSync(DEST);
  fs.cpSync('./tests', DEST, { recursive: true });
}
