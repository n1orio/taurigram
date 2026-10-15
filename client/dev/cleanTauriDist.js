// Removes build reports and source maps from `dist/` before it is bundled into the Tauri app
import { readdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';

const DIST_DIR = join(import.meta.dirname, '..', 'dist');
const REPORT_FILES = ['build-stats.json', 'statoscope-report.html'];

for (const name of readdirSync(DIST_DIR)) {
  if (REPORT_FILES.includes(name) || name.endsWith('.map')) {
    rmSync(join(DIST_DIR, name), { force: true });
  }
}
