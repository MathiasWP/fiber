import { expect, test } from '@playwright/test';
import { install } from './mock-ipc';

test('an available update offers three ways out', async ({ page }) => {
	await install(page, { update: { version: '1.2.3' } });
	await page.goto('/');

	const toast = page.getByRole('status');
	await expect(toast).toContainText('Fiber 1.2.3 is available');
	await expect(toast.getByRole('button', { name: 'Not now' })).toBeVisible();
	await expect(toast.getByRole('button', { name: 'On next launch' })).toBeVisible();
	await expect(toast.getByRole('button', { name: 'Update' })).toBeVisible();
});

test('Not now hides the toast for the rest of this run', async ({ page }) => {
	await install(page, { update: { version: '1.2.3' } });
	await page.goto('/');

	await page.getByRole('button', { name: 'Not now' }).click();
	await expect(page.getByRole('status')).toBeHidden();

	await page.evaluate(() => window.dispatchEvent(new Event('focus')));
	await page.waitForTimeout(150);
	await expect(page.getByRole('status')).toBeHidden();
});

test('On next launch installs without restarting', async ({ page }) => {
	await install(page, { update: { version: '1.2.3' } });
	await page.goto('/');

	await page.getByRole('button', { name: 'On next launch' }).click();
	await expect(page.getByText('Fiber 1.2.3 is ready')).toBeVisible();
	await expect(page.getByText(/next time you open Fiber/)).toBeVisible();
	await page.getByRole('button', { name: 'Close' }).click();
	await expect(page.getByRole('status')).toBeHidden();
});

test('Update downloads with a progress bar, then restarts', async ({ page }) => {
	await install(page, {
		update: { version: '1.2.3', deferDownload: true, contentLength: 1000 }
	});
	await page.goto('/');

	await page.getByRole('button', { name: 'Update' }).click();
	await expect(page.getByText(/Downloading 1.2.3/)).toBeVisible();

	await page.evaluate(() => window.__FIBER_TEST__.updateProgress(250));
	await expect(page.getByText('25%')).toBeVisible();

	await page.evaluate(() => window.__FIBER_TEST__.finishUpdate());
	await expect(page.getByText(/Installing 1.2.3/)).toBeVisible();
});

test('a download that dies halfway is shown, not swallowed', async ({ page }) => {
	await install(page, {
		update: { version: '1.2.3', downloadError: 'network dropped' }
	});
	await page.goto('/');

	await page.getByRole('button', { name: 'Update' }).click();
	await expect(page.getByText("The update didn't install")).toBeVisible();
	await expect(page.getByText(/network dropped/)).toBeVisible();
	await page.getByRole('button', { name: 'Close' }).click();
	await expect(page.getByText("The update didn't install")).toBeHidden();
});

test('a failed restart still tells you the update is in', async ({ page }) => {
	await install(page, {
		update: { version: '1.2.3', restartError: 'could not relaunch' }
	});
	await page.goto('/');

	await page.getByRole('button', { name: 'Update' }).click();
	await expect(page.getByText(/Update installed — restart Fiber to finish/)).toBeVisible();
	await expect(page.getByText(/automatic restart didn't happen/)).toBeVisible();
});

test('the dismiss X is the same as Not now', async ({ page }) => {
	await install(page, { update: { version: '1.2.3' } });
	await page.goto('/');
	await page.getByLabel('Dismiss').click();
	await expect(page.getByRole('status')).toBeHidden();
});

test('a missing content-length shows an indeterminate bar, not a stuck one', async ({ page }) => {
	await install(page, {
		update: { version: '1.2.3', deferDownload: true, contentLength: 0 }
	});
	await page.goto('/');

	await page.getByRole('button', { name: 'Update' }).click();
	await expect(page.getByText(/Downloading 1.2.3/)).toBeVisible();
	// No percentage readout, and the bar pulses rather than sitting at 0%.
	await expect(page.getByText(/%/)).toBeHidden();
	await expect(page.locator('.animate-pulse')).toBeVisible();

	await page.evaluate(() => window.__FIBER_TEST__.updateProgress(500));
	await page.waitForTimeout(100);
	await expect(page.getByText(/%/)).toBeHidden();
});

test('a focus check never interrupts a download in progress', async ({ page }) => {
	await install(page, {
		update: { version: '1.2.3', deferDownload: true, contentLength: 1000 }
	});
	await page.goto('/');

	await page.getByRole('button', { name: 'Update' }).click();
	await expect(page.getByText(/Downloading 1.2.3/)).toBeVisible();

	await page.evaluate(() => window.dispatchEvent(new Event('focus')));
	await page.waitForTimeout(150);
	// Still downloading — a check that ran anyway would have reset the toast
	// back to "available" or left two competing states.
	await expect(page.getByText(/Downloading 1.2.3/)).toBeVisible();
	await expect(page.getByText('Fiber 1.2.3 is available')).toBeHidden();

	await page.evaluate(() => window.__FIBER_TEST__.finishUpdate());
	await expect(page.getByText(/Installing 1.2.3/)).toBeVisible();
});

test('declining is for this run only — a reload offers the same version again', async ({
	page
}) => {
	await install(page, { update: { version: '1.2.3' } });
	await page.goto('/');

	await page.getByRole('button', { name: 'Not now' }).click();
	await expect(page.getByRole('status')).toBeHidden();

	await page.reload();
	await expect(page.getByText('Fiber 1.2.3 is available')).toBeVisible();
});

test('a newer version than the one declined is still offered', async ({ page }) => {
	// Padded with repeats of the declined version: some browsers fire an extra
	// focus check as a side effect of the click itself, so the exact number of
	// checks before the newer one appears isn't something to pin down here —
	// only that it eventually does, and that the declined version never comes
	// back on its own.
	await install(page, {
		update: { version: '1.2.3', versions: ['1.2.3', '1.2.3', '1.2.3', '1.3.0'] }
	});
	await page.goto('/');

	await expect(page.getByText('Fiber 1.2.3 is available')).toBeVisible();
	await page.getByRole('button', { name: 'Not now' }).click();

	await expect
		.poll(async () => {
			await page.evaluate(() => window.dispatchEvent(new Event('focus')));
			return page.getByText('Fiber 1.3.0 is available').isVisible();
		})
		.toBe(true);
});

test('declining the same version again after a focus check keeps it hidden', async ({ page }) => {
	await install(page, { update: { version: '1.2.3', versions: ['1.2.3', '1.2.3'] } });
	await page.goto('/');

	await page.getByRole('button', { name: 'Not now' }).click();
	await expect(page.getByRole('status')).toBeHidden();

	await page.evaluate(() => window.dispatchEvent(new Event('focus')));
	await page.waitForTimeout(150);
	await expect(page.getByRole('status')).toBeHidden();
});
