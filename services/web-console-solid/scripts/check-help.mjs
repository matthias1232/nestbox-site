// Validates the help folder. Wave 0 only checks that public/help/index.json parses and is a list;
// package F18 adds the front matter, link and image checks and generates the index.
import { readFile } from 'node:fs/promises';

const file = new URL('../public/help/index.json', import.meta.url);
let parsed;
try {
  parsed = JSON.parse(await readFile(file, 'utf8'));
} catch (error) {
  console.error(`check-help: public/help/index.json does not parse: ${error.message}`);
  process.exit(1);
}
if (!Array.isArray(parsed)) {
  console.error('check-help: public/help/index.json must be a JSON array');
  process.exit(1);
}
console.log(`check-help: ${parsed.length} document(s) indexed`);
