import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';

/**
 * The fake backend's weak spot: it can drift from the real Rust commands and
 * every UI test still passes, because nothing on this side would notice.
 *
 * This catches the direction drift actually comes from — a command renamed or
 * removed in `api.ts`, leaving the mock answering a name nothing asks for any
 * more. `loader_examples` becoming `loader_templates` is exactly that shape.
 *
 * It deliberately does not require every command to be mocked. The mock throws
 * on an unknown command, so a missing one already fails loudly, at the test that
 * needed it, with the name in the message — which is more use than a list here.
 *
 * It cannot see a changed *argument or return shape*. Only running against the
 * real binary would, which is what a `@wdio/tauri-service` layer would be for.
 */

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const read = (path: string) => readFileSync(join(root, path), 'utf8');

/** Commands the frontend actually invokes. */
function invoked(): Set<string> {
	const names = read('src/lib/api.ts').matchAll(/invoke<[^>]*>\(\s*'([^']+)'/g);
	return new Set(Array.from(names, (match) => match[1]));
}

/** Commands the mock answers. */
function mocked(): Set<string> {
	const names = read('tests/e2e/mock-ipc.ts').matchAll(/case '([^']+)':/g);
	return new Set(Array.from(names, (match) => match[1]));
}

test('the mock only answers commands the app still invokes', () => {
	const real = invoked();
	const fake = mocked();

	// Guard the guard: a regex that stopped matching would make every assertion
	// below pass while checking nothing.
	expect(real.size).toBeGreaterThan(20);
	expect(fake.size).toBeGreaterThan(5);

	// Tauri's own plugins are invoked from inside @tauri-apps/api rather than
	// from api.ts, so they will never appear in `real`.
	const plugins = [...fake].filter((name) => name.startsWith('plugin:'));
	expect(plugins.length).toBeGreaterThan(0);

	const stale = [...fake].filter((name) => !name.startsWith('plugin:') && !real.has(name));
	expect(stale, 'mocked commands that no longer exist in api.ts').toEqual([]);
});

test('the mock covers what the tests currently reach', () => {
	// Not every command — see the note above — but the startup path has to be
	// covered or nothing renders at all, and that failure is confusing.
	const fake = mocked();
	for (const command of ['list_sections', 'history_list', 'plugin:app|version']) {
		expect(fake, `startup needs ${command}`).toContain(command);
	}
});
