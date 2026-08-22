import { expect, test } from '@playwright/test';
import { commands, historyRecord, install, savedRequest, section } from './mock-ipc';

const users = savedRequest();

test('the History tab is empty until something has been sent', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await page.getByRole('button', { name: 'History' }).click();
	await expect(page.getByText(/No requests yet/)).toBeVisible();
});

test('a sent request appears in History with its name', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	await page.goto('/');

	await page.getByText('List users').click();
	await page.getByRole('button', { name: 'Send' }).click();
	await page.getByRole('button', { name: 'History' }).click();

	await expect(page.getByText('List users')).toBeVisible();
	await expect(page.getByText('200', { exact: true })).toBeVisible();
	await expect(page.getByText('https://api.acme.com/users')).toBeVisible();
});

test.describe('opening an entry', () => {
	test('loads the saved body and does not overwrite the request\'s current response', async ({
		page
	}) => {
		await install(page, {
			sections: [section({ requests: [users] })],
			history: [
				historyRecord({
					id: 'h-new',
					at: 2_000,
					response: {
						status: 200,
						statusText: 'OK',
						finalUrl: 'https://api.acme.com/users',
						headers: [{ name: 'content-type', value: 'application/json' }],
						isBinary: false,
						truncated: false,
						sizeBytes: 5,
						timing: { ttfbMs: 1, totalMs: 2 }
					}
				}),
				historyRecord({
					id: 'h-old',
					at: 1_000,
					response: {
						status: 404,
						statusText: 'Not Found',
						finalUrl: 'https://api.acme.com/users',
						headers: [{ name: 'content-type', value: 'application/json' }],
						isBinary: false,
						truncated: false,
						sizeBytes: 5,
						timing: { ttfbMs: 1, totalMs: 2 }
					}
				})
			],
			historyBodies: { 'h-new': '{"now":true}', 'h-old': '{"then":true}' }
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await expect(page.getByText('200')).toBeVisible();
		await expect(page.locator('.cm-content:visible').last()).toContainText('"now"');

		await page.getByRole('button', { name: 'History' }).click();
		await page.getByText('404', { exact: true }).click();
		await expect(page.getByText('404', { exact: true })).toBeVisible();
		await expect(page.locator('.cm-content:visible').last()).toContainText('"then"');

		await page.getByRole('button', { name: 'Collections' }).click();
		await expect(page.getByText('200')).toBeVisible();
		await expect(page.locator('.cm-content:visible').last()).toContainText('"now"');
	});

	test('an entry whose request is gone loads into scratch', async ({ page }) => {
		await install(page, {
			sections: [section()],
			history: [
				historyRecord({
					id: 'h-orphan',
					requestId: 'gone',
					method: 'PATCH',
					url: 'https://api.acme.com/retired',
					requestBody: '{"revive":true}'
				})
			],
			historyBodies: { 'h-orphan': '{"gone":true}' }
		});
		await page.goto('/');
		await page.getByRole('button', { name: 'History' }).click();
		await page.getByText('https://api.acme.com/retired').click();

		await expect(page.getByPlaceholder('https://api.example.com/users')).toHaveValue(
			'https://api.acme.com/retired'
		);
		await expect(page.getByLabel('HTTP method')).toContainText('PATCH');
		await expect(page.locator('.cm-content').first()).toContainText('"revive"');
		await expect(page.locator('.cm-content:visible').last()).toContainText('"gone"');
	});
});

test.describe('searching history', () => {
	test('matches name, method, URL and status', async ({ page }) => {
		await install(page, {
			sections: [section({ requests: [users] })],
			history: [
				historyRecord({ id: 'h1', url: 'https://api.acme.com/users' }),
				historyRecord({
					id: 'h2',
					requestId: 'r2',
					method: 'POST',
					url: 'https://api.acme.com/orders',
					response: {
						status: 201,
						statusText: 'Created',
						finalUrl: 'https://api.acme.com/orders',
						headers: [],
						isBinary: false,
						truncated: false,
						sizeBytes: 0,
						timing: { ttfbMs: 1, totalMs: 2 }
					}
				})
			]
		});
		await page.goto('/');
		await page.getByRole('button', { name: 'History' }).click();

		const search = page.getByPlaceholder('Search history…');
		await search.fill('404');
		await expect(page.getByText('Nothing matches “404”.')).toBeVisible();

		await search.fill('201');
		await expect(page.getByText('https://api.acme.com/orders')).toBeVisible();
		await expect(page.getByText('https://api.acme.com/users')).toBeHidden();

		await search.fill('POST');
		await expect(page.getByText('https://api.acme.com/orders')).toBeVisible();

		await search.fill('List users');
		await expect(page.getByText('https://api.acme.com/users')).toBeVisible();
		await expect(page.getByText('https://api.acme.com/orders')).toBeHidden();
	});
});

test('Show all reveals the rest of a long list', async ({ page }) => {
	const history = Array.from({ length: 101 }, (_, i) =>
		historyRecord({
			id: `h-${i}`,
			at: 2_000 - i,
			url: `https://api.acme.com/users/${i}`
		})
	);
	await install(page, { history });
	await page.goto('/');
	await page.getByRole('button', { name: 'History' }).click();

	await expect(page.getByText('https://api.acme.com/users/0')).toBeVisible();
	await expect(page.getByText('https://api.acme.com/users/100')).toBeHidden();
	await page.getByRole('button', { name: /Show all 101 entries/ }).click();
	await expect(page.getByText('https://api.acme.com/users/100')).toBeVisible();
});

test.describe('removing history', () => {
	test('Remove entry deletes one row', async ({ page }) => {
		await install(page, {
			history: [
				historyRecord({ id: 'h1', url: 'https://api.acme.com/one' }),
				historyRecord({ id: 'h2', url: 'https://api.acme.com/two' })
			]
		});
		await page.goto('/');
		await page.getByRole('button', { name: 'History' }).click();

		await page.getByText('https://api.acme.com/one').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Remove entry' }).click();
		await expect(page.getByText('https://api.acme.com/one')).toBeHidden();
		await expect(page.getByText('https://api.acme.com/two')).toBeVisible();
		await expect.poll(() => commands(page, 'history_delete')).not.toEqual([]);
	});

	test('a failed delete puts the row back', async ({ page }) => {
		await install(page, {
			history: [historyRecord()],
			deleteHistoryError: 'locked'
		});
		await page.goto('/');
		await page.getByRole('button', { name: 'History' }).click();

		await page.getByText('https://api.acme.com/users').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Remove entry' }).click();
		await expect(page.getByText('https://api.acme.com/users')).toBeVisible();
		await expect(page.getByText(/locked/)).toBeVisible();
	});

	test('the trash button clears everything', async ({ page }) => {
		await install(page, {
			history: [
				historyRecord({ id: 'h1' }),
				historyRecord({ id: 'h2', url: 'https://api.acme.com/other' })
			]
		});
		await page.goto('/');
		await page.getByRole('button', { name: 'History' }).click();
		await page.getByTitle('Clear history').click();
		await expect(page.getByText(/No requests yet/)).toBeVisible();
		await expect.poll(() => commands(page, 'history_clear_all')).not.toEqual([]);
	});

	test('Copy URL from an entry writes it', async ({ page }) => {
		await install(page, { history: [historyRecord()] });
		await page.goto('/');
		await page.getByRole('button', { name: 'History' }).click();

		await page.getByText('https://api.acme.com/users').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Copy URL' }).click();
		await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(
			'https://api.acme.com/users'
		);
	});
});
