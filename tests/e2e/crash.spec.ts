import { expect, test, type Page } from '@playwright/test';
import { install } from './mock-ipc';

/**
 * Chromium does not deliver a synthetic `ErrorEvent` to `<svelte:window
 * onerror>` the way a real throw does, and Playwright's own error plumbing
 * swallows `page.evaluate` rejections before they become `unhandledrejection`.
 * The app registers the same `record` path on `__FIBER_TEST__` once it mounts.
 */
async function explode(page: Page, message: string) {
	await page.waitForFunction(
		() => !window.__FIBER_TEST__.crash.toString().includes('has not registered')
	);
	await page.evaluate((text) => window.__FIBER_TEST__.crash(text), message);
}

test('an uncaught error is named on screen, with the version', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await explode(page, 'something exploded');

	const banner = page.getByRole('alert');
	await expect(banner).toContainText('something exploded');
	await expect(banner).toContainText('Fiber 0.0.0-test');
});

test('the first error wins, so a loop cannot bury the cause', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await explode(page, 'first');
	await explode(page, 'second');

	await expect(page.getByRole('alert')).toContainText('first');
	await expect(page.getByRole('alert')).not.toContainText('second');
});

test('Copy puts the report on the clipboard, and Hide dismisses it', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await explode(page, 'copy me');

	await page.getByRole('button', { name: 'Copy' }).click();
	await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('copy me');
	await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain(
		'Fiber 0.0.0-test'
	);

	await page.getByRole('button', { name: 'Hide' }).click();
	await expect(page.getByRole('alert')).toBeHidden();
});

test('the banner is usable while a dialog is open', async ({ page }) => {
	await install(page);
	await page.goto('/');

	// A modal dialog neutralises pointer events on `<body>` and closes itself on
	// any pointerdown outside it. The banner has to sit above both.
	await page.waitForFunction(
		() => !window.__FIBER_TEST__.openPalette.toString().includes('has not registered')
	);
	await page.evaluate(() => window.__FIBER_TEST__.openPalette());
	const palette = page.getByRole('dialog', { name: 'Search endpoints' });
	await expect(palette).toBeVisible();

	await explode(page, 'broke mid-dialog');

	await page.getByRole('button', { name: 'Hide' }).click();
	await expect(page.getByRole('alert')).toBeHidden();
	// The click pressed the button rather than dismissing what was underneath.
	await expect(palette).toBeVisible();
});

test('an unhandled rejection is reported the same way', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await page.waitForFunction(
		() => !window.__FIBER_TEST__.reject.toString().includes('has not registered')
	);
	await page.evaluate(() => window.__FIBER_TEST__.reject('background died'));

	await expect(page.getByRole('alert')).toContainText('A background task failed');
	await expect(page.getByRole('alert')).toContainText('background died');
});
