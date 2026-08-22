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
		return field;
	}

	const selection = (field: import('@playwright/test').Locator) =>
		field.evaluate((node: HTMLInputElement) => [node.selectionStart, node.selectionEnd]);

	test('Shift+Alt+ArrowRight extends the selection to the next boundary', async ({ page }) => {
		const field = await urlField(page);
		await field.evaluate((node: HTMLInputElement) => node.setSelectionRange(8, 8)); // after "https://"
		await field.press('Shift+Alt+ArrowRight');
		// Selection grows to cover "app", anchored where the caret started.
		expect(await selection(field)).toEqual([8, 11]);
	});

	test('Shift+Alt+ArrowLeft from an existing forward selection shrinks it, not the far end', async ({
		page
	}) => {
		const field = await urlField(page);
		// Select "app.staging" (8..19), caret (focus) at the end, anchor at 8.
		await field.evaluate((node: HTMLInputElement) => node.setSelectionRange(8, 19, 'forward'));
		await field.press('Shift+Alt+ArrowLeft');
		// The focus end retreats one boundary; the anchor at 8 is untouched.
		expect(await selection(field)).toEqual([8, 12]);
	});

	test('plain ArrowRight without Alt is left to the browser, landing right after the caret', async ({
		page
	}) => {
		const field = await urlField(page);
		await field.evaluate((node: HTMLInputElement) => node.setSelectionRange(0, 0));
		await field.press('ArrowRight');
		expect(await selection(field)).toEqual([1, 1]);
	});

	test('Ctrl+Alt+ArrowRight is not intercepted — an extra modifier opts out', async ({ page }) => {
		const field = await urlField(page);
		await field.evaluate((node: HTMLInputElement) => node.setSelectionRange(0, 0));
		await field.press('Control+Alt+ArrowRight');
		// Whatever the browser does with this combination, it isn't the
		// word-jump — the value is untouched either way.
		await expect(field).toHaveValue('https://app.staging.example.com');
	});

	test('Meta+Alt+ArrowRight is not intercepted either', async ({ page }) => {
		const field = await urlField(page);
		await field.evaluate((node: HTMLInputElement) => node.setSelectionRange(0, 0));
		await field.press('Meta+Alt+ArrowRight');
		await expect(field).toHaveValue('https://app.staging.example.com');
	});

	test('Alt+ArrowLeft at position 0 does nothing, rather than throwing', async ({ page }) => {
		const field = await urlField(page);
		await field.evaluate((node: HTMLInputElement) => node.setSelectionRange(0, 0));
		await field.press('Alt+ArrowLeft');
		expect(await selection(field)).toEqual([0, 0]);
	});

	test('Alt+ArrowRight at the end does nothing, rather than throwing', async ({ page }) => {
		const field = await urlField(page);
		const end = 'https://app.staging.example.com'.length;
		await field.evaluate((node: HTMLInputElement, e: number) => node.setSelectionRange(e, e), end);
		await field.press('Alt+ArrowRight');
		expect(await selection(field)).toEqual([end, end]);
	});
});
