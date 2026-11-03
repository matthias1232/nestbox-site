// A guard so German cannot creep back into the console.
//
// Rule: every .js and .jsx file under src/ that is not a test is read line by
// line. A line fails when it contains one of the German characters
// ä ö ü Ä Ö Ü ß, or one of the German words the P08 review names: Keine, Kein,
// Nicht, Noch, Blockiert, Anfrage, gewählt and zurücksetzen. Two kinds of line
// are ignored: a line that is a comment (its first non-space characters open
// `//`, `/*` or `*`), and a line that mentions "AGENTS", which is a path or a
// note rather than text a reader sees. The check does not parse JavaScript: a
// word inside a string is treated like code, because the console ships English
// and a German word in a string is exactly what this test is about. Test files
// and *.md are out of scope, because a fixture may hold German source text.
//
// The check is deliberately simple and honest about its limits: it catches the
// characters and the words above, not every German word without an umlaut.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('.', import.meta.url));
const GERMAN =
  /[äöüÄÖÜß]|\b(?:Keine|Kein|Nicht|Noch|Blockiert|Anfrage|gewählt|zurücksetzen)\b/;

/** Every source file the rule covers: .js and .jsx, but no tests. */
function sources() {
  return readdirSync(SRC, { recursive: true })
    .map((name) => String(name))
    .filter((name) => /\.(?:js|jsx)$/.test(name))
    .filter((name) => !/\.test\.(?:js|jsx)$/.test(name));
}

/** True for a comment line, or a line that only carries an AGENTS reference. */
function ignored(line) {
  const bare = line.trim();
  return (
    bare.startsWith('//') ||
    bare.startsWith('/*') ||
    bare.startsWith('*') ||
    line.includes('AGENTS')
  );
}

test('the console speaks English: no German characters or words left', () => {
  const hits = [];
  for (const name of sources()) {
    const lines = readFileSync(join(SRC, name), 'utf8').split('\n');
    lines.forEach((line, index) => {
      if (ignored(line)) return;
      if (GERMAN.test(line)) hits.push(`${name}:${index + 1}: ${line.trim()}`);
    });
  }
  assert.deepEqual(hits, [], `German text is back in the console:\n${hits.join('\n')}`);
});
