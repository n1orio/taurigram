// Removes build reports and source maps from `dist/` before it is bundled into the Tauri app
import { readdirSync, rmSync } from 'node:fs';
import { basename, join } from 'node:path';

const DIST_DIR = join(import.meta.dirname, '..', 'dist');
const REPORT_FILES = ['build-stats.json', 'statoscope-report.html'];

for (const path of readdirSync(DIST_DIR, { recursive: true })) {
  if (REPORT_FILES.includes(path) || basename(path).endsWith('.map')) {
    rmSync(join(DIST_DIR, path), { force: true });
  }
}
