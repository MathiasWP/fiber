import { expect, test } from '@playwright/test';
import { install } from './mock-ipc';

/**
 * `collections.spec.ts` covers the footer's click-to-flip and its context
 * menu. This covers the `system` mode itself — following `matchMedia`,
 * ignoring it once you've picked a side, and persistence across a reload.
 */

function currentTheme(page: import('@playwright/test').Page) {
	return page.evaluate(() => document.documentElement.dataset.theme);
}

test('a fresh install follows the OS light/dark preference', async ({ page }) => {
	await page.emulateMedia({ colorScheme: 'light' });
	await install(page);
	await page.goto('/');
	await expect.poll(() => currentTheme(page)).toBe('light');

	await page.emulateMedia({ colorScheme: 'dark' });
	await expect.poll(() => currentTheme(page)).toBe('dark');
});

test('changing the OS preference updates the app live while following the system', async ({
	page
}) => {
	await page.emulateMedia({ colorScheme: 'dark' });
	await install(page);
	await page.goto('/');
	await expect.poll(() => currentTheme(page)).toBe('dark');

	await page.emulateMedia({ colorScheme: 'light' });
	await expect.poll(() => currentTheme(page)).toBe('light');

	await page.emulateMedia({ colorScheme: 'dark' });
	await expect.poll(() => currentTheme(page)).toBe('dark');
});

test('picking a side stops the OS preference from having any further effect', async ({ page }) => {
	await page.emulateMedia({ colorScheme: 'light' });
	await install(page);
	await page.goto('/');
	await expect.poll(() => currentTheme(page)).toBe('light');

	// Flipping from the footer picks the opposite of what's resolved now.
	await page.getByTitle(/Switch to/).click();
	await expect.poll(() => currentTheme(page)).toBe('dark');

	// The OS switching back to light should not undo the explicit choice.
	await page.emulateMedia({ colorScheme: 'light' });
	await page.waitForTimeout(200);
	expect(await currentTheme(page)).toBe('dark');
});

test('"Follow the system" hands control back to the OS preference', async ({ page }) => {
	await page.emulateMedia({ colorScheme: 'light' });
	await install(page);
	await page.goto('/');

	const trigger = page.getByTitle(/Switch to/);
	await trigger.click();
	await expect.poll(() => currentTheme(page)).toBe('dark');

	await trigger.click({ button: 'right' });
	await page.getByRole('menuitem', { name: 'Follow the system' }).click();
	await expect.poll(() => currentTheme(page)).toBe('light');

	// Now that it's following again, the OS can change it.
	await page.emulateMedia({ colorScheme: 'dark' });
	await expect.poll(() => currentTheme(page)).toBe('dark');
});

test('an explicit choice survives a reload, and does not fall back to the system', async ({
	page
}) => {
	await page.emulateMedia({ colorScheme: 'light' });
	await install(page);
	await page.goto('/');
	await page.getByTitle(/Switch to/).click();
	await expect.poll(() => currentTheme(page)).toBe('dark');

	await page.reload();
	await expect.poll(() => currentTheme(page)).toBe('dark');

	// The OS is still light — a system fallback would have shown that instead.
	await page.emulateMedia({ colorScheme: 'dark' });
	await page.waitForTimeout(100);
	expect(await currentTheme(page)).toBe('dark');
});
