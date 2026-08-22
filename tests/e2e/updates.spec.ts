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
