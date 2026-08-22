import { expect, test } from '@playwright/test';
import { backward, forward } from '../../src/lib/urlfield';
import { install } from './mock-ipc';

/**
 * `forward`/`backward` are pure boundary logic, tested directly the same way
 * `search.spec.ts` tests `fuzzyScore`. `app.spec.ts` already covers the basic
 * Alt+Arrow interaction; this adds the boundary cases for the functions
 * themselves, plus the selection-extending (Shift) and modifier-guard
 * behaviour that only shows up wired into a real field.
 */

test.describe('forward', () => {
	test('from the very start, skips the scheme joint and lands after the first word', () => {
		expect(forward('https://api.example.com', 0)).toBe(5); // "https"
	});

	test('at the end of the string, stays at the end', () => {
		const value = 'https://api.example.com';
		expect(forward(value, value.length)).toBe(value.length);
	});

	test('consecutive joints are all skipped in one jump', () => {
		// "://" between "https" and "api" is three joints, not three stops.
		expect(forward('https://api', 5)).toBe(11);
	});

	test('an empty string has nowhere to go', () => {
		expect(forward('', 0)).toBe(0);
	});

	test('a string of only joints moves to the end', () => {
		expect(forward('://///', 0)).toBe(6);
	});

	test('digits count as part of a word, not a joint', () => {
		expect(forward('port:8080/path', 5)).toBe(9);
	});
});

test.describe('backward', () => {
	test('from the very end, skips the trailing joint and lands before the last word', () => {
		const value = 'https://api.example.com';
		expect(backward(value, value.length)).toBe(value.length - 3); // "com"
	});

	test('at the start of the string, stays at the start', () => {
		expect(backward('https://api.example.com', 0)).toBe(0);
	});

	test('consecutive joints are all skipped in one jump', () => {
		expect(backward('https://api', 8)).toBe(0);
	});

	test('an empty string has nowhere to go', () => {
		expect(backward('', 0)).toBe(0);
	});

	test('forward then backward from the same spot returns to it, across a joint', () => {
		const value = 'https://api.example.com';
		const boundary = forward(value, 0);
		expect(backward(value, boundary)).toBe(0);
	});
});

test.describe('wired into a real field', () => {
	async function urlField(page: import('@playwright/test').Page) {
		await install(page);
		await page.goto('/');
		await page.locator('button', { has: page.locator('.i-lucide-folder-plus') }).first().click();

		const field = page.getByPlaceholder('https://api.example.com', { exact: true });
		await expect(field).toBeVisible();
		await field.fill('https://app.staging.example.com');
		// `fill` returns once the DOM value is set and its events dispatched,
		// but Svelte's own reactive write-back of that value (via `bind:value`)
		// can still be pending under load, and can otherwise land in between
		// this and a later `setSelectionRange` call, clobbering the selection
		// it just set. Settling here — past a repaint, and past the write-back
		// — is what keeps that race out of every test below.
		await field.evaluate(() => new Promise(requestAnimationFrame));
		await expect(field).toHaveValue('https://app.staging.example.com');
		return field;
	}

	/**
	 * Sets the selection and dispatches the keydown in one round trip to the
	 * page. The attachment moves the caret itself (`preventDefault` plus a
	 * manual `setSelectionRange`) rather than relying on the browser's native
	 * handling, so a script-dispatched event exercises it exactly the same as
	 * a real keypress — without the two separate `evaluate`/`press` calls
	 * leaving a gap where Svelte's own reactivity could touch the field's
	 * selection in between, and without routing modifier combinations like
	 * Ctrl+Alt or Meta+Alt through `page.keyboard`, where a window manager
	 * could intercept them as real shortcuts before the page ever sees them.
	 */
	async function pressAt(
		field: import('@playwright/test').Locator,
		start: number,
		end: number,
		key: 'ArrowLeft' | 'ArrowRight',
		modifiers: { shiftKey?: boolean; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean } = {
			altKey: true
		},
		direction?: 'forward' | 'backward'
	): Promise<{ selection: [number | null, number | null]; defaultPrevented: boolean }> {
		return field.evaluate(
			(
				node: HTMLInputElement,
				arg: {
					start: number;
					end: number;
					key: string;
					modifiers: typeof modifiers;
					direction?: 'forward' | 'backward';
				}
			) => {
				node.focus();
				node.setSelectionRange(arg.start, arg.end, arg.direction);
				const event = new KeyboardEvent('keydown', {
					key: arg.key,
					bubbles: true,
					cancelable: true,
					...arg.modifiers
				});
				node.dispatchEvent(event);
				return {
					selection: [node.selectionStart, node.selectionEnd] as [number | null, number | null],
					defaultPrevented: event.defaultPrevented
				};
			},
			{ start, end, key, modifiers, direction }
		);
	}

	test('Shift+Alt+ArrowRight extends the selection to the next boundary', async ({ page }) => {
		const field = await urlField(page);
		const result = await pressAt(field, 8, 8, 'ArrowRight', { shiftKey: true, altKey: true });
		// Selection grows to cover "app", anchored where the caret started.
		expect(result.selection).toEqual([8, 11]);
	});

	test('Shift+Alt+ArrowLeft from an existing forward selection shrinks it, not the far end', async ({
		page
	}) => {
		const field = await urlField(page);
		// Select "app.staging" (8..19), caret (focus) at the end, anchor at 8.
		const result = await pressAt(
			field,
			8,
			19,
			'ArrowLeft',
			{ shiftKey: true, altKey: true },
			'forward'
		);
		// The focus end retreats one boundary; the anchor at 8 is untouched.
		expect(result.selection).toEqual([8, 12]);
	});

	test('plain ArrowRight without Alt is left alone — the guard does not preventDefault', async ({
		page
	}) => {
		const field = await urlField(page);
		const result = await pressAt(field, 0, 0, 'ArrowRight', {});
		expect(result.defaultPrevented).toBe(false);
	});

	test('Ctrl+Alt+ArrowRight is not intercepted — an extra modifier opts out', async ({ page }) => {
		const field = await urlField(page);
		const result = await pressAt(field, 0, 0, 'ArrowRight', { ctrlKey: true, altKey: true });
		expect(result.defaultPrevented).toBe(false);
	});

	test('Meta+Alt+ArrowRight is not intercepted either', async ({ page }) => {
		const field = await urlField(page);
		const result = await pressAt(field, 0, 0, 'ArrowRight', { metaKey: true, altKey: true });
		expect(result.defaultPrevented).toBe(false);
	});

	test('Alt+ArrowLeft at position 0 does nothing, rather than throwing', async ({ page }) => {
		const field = await urlField(page);
		const result = await pressAt(field, 0, 0, 'ArrowLeft', { altKey: true });
		expect(result.selection).toEqual([0, 0]);
	});

	test('Alt+ArrowRight at the end does nothing, rather than throwing', async ({ page }) => {
		const field = await urlField(page);
		const end = 'https://app.staging.example.com'.length;
		const result = await pressAt(field, end, end, 'ArrowRight', { altKey: true });
		expect(result.selection).toEqual([end, end]);
	});
});
