import { expect, test } from '@playwright/test';
import { install, response, savedRequest, section } from './mock-ipc';

const users = savedRequest();

test('a request with no history asks you to send one', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	await page.goto('/');
	await page.getByText('List users').click();
	await expect(page.getByText('Send a request to see the response.')).toBeVisible();
});

test.describe('a settled response', () => {
	test('Pretty formats JSON, Raw does not, Headers lists them', async ({ page }) => {
		await install(page, {
			sendResponse: response({
				body: '{"hello":"world"}',
				sizeBytes: 17,
				headers: [
					{ name: 'content-type', value: 'application/json' },
					{ name: 'x-request-id', value: 'abc' }
				]
			}),
			sections: [section({ requests: [users] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();

		await expect(page.getByText('200 OK')).toBeVisible();
		await expect(page.locator('.cm-content').nth(1)).toContainText('"hello": "world"');

		await page.getByRole('tab', { name: 'Raw' }).click();
		await expect(page.locator('.cm-content').nth(1)).toContainText('{"hello":"world"}');

		await page.getByRole('tab', { name: 'Headers (2)' }).click();
		await expect(page.getByText('content-type', { exact: true })).toBeVisible();
		await expect(page.getByText('x-request-id')).toBeVisible();
		await expect(page.getByText('abc')).toBeVisible();
	});

	test('a redirect is named on the headers tab', async ({ page }) => {
		await install(page, {
			sendResponse: response({
				finalUrl: 'https://api.acme.com/v2/users',
				headers: [{ name: 'location', value: '/v2/users' }]
			}),
			sections: [section({ requests: [users] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();
		await page.getByRole('tab', { name: 'Headers (1)' }).click();
		await expect(page.getByText(/Redirected to/)).toBeVisible();
		await expect(page.getByText('https://api.acme.com/v2/users')).toBeVisible();
	});

	test('a truncated body is flagged rather than looking complete', async ({ page }) => {
		await install(page, {
			sendResponse: response({ truncated: true, sizeBytes: 32 * 1024 * 1024 }),
			sections: [section({ requests: [users] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();
		await expect(page.getByText(/Response truncated at 32 MB/)).toBeVisible();
	});

	test('a binary body is not dumped into Pretty', async ({ page }) => {
		await install(page, {
			sendResponse: response({
				isBinary: true,
				body: 'AAAA',
				sizeBytes: 3,
				headers: [{ name: 'content-type', value: 'application/octet-stream' }]
			}),
			sections: [section({ requests: [users] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();
		await expect(page.getByText(/Binary response/)).toBeVisible();

		await page.getByRole('tab', { name: 'Raw' }).click();
		await expect(page.locator('.cm-content').nth(1)).toContainText('AAAA');
	});

	test('Pretty is disabled when the body is too large to format', async ({ page }) => {
		const body = `{"pad":"${'x'.repeat(1.6 * 1024 * 1024)}"}`;
		await install(page, {
			sendResponse: response({
				body,
				sizeBytes: body.length,
				headers: [{ name: 'content-type', value: 'application/json' }]
			}),
			sections: [section({ requests: [users] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();

		const pretty = page.getByRole('tab', { name: 'Pretty' });
		await expect(pretty).toBeDisabled();
		await expect(pretty).toHaveAttribute('title', /Too large to pretty-print/);
	});
});

test('waiting for a reply occupies the pane until something arrives', async ({ page }) => {
	await install(page, {
		deferSend: true,
		sections: [section({ requests: [users] })]
	});
	await page.goto('/');
	await page.getByText('List users').click();
	await page.getByRole('button', { name: 'Send' }).click();

	await expect(page.getByText('Send a request to see the response.')).toBeHidden();
	await expect(page.getByText('Streaming')).toBeHidden();
	await expect(page.getByRole('button', { name: 'Cancel' })).toBeVisible();
});

test('a 401 retry drops the first attempt\'s body rather than concatenating', async ({ page }) => {
	await install(page, {
		deferSend: true,
		sections: [section({ requests: [users] })]
	});
	await page.goto('/');
	await page.getByText('List users').click();
	await page.getByRole('button', { name: 'Send' }).click();

	await page.evaluate(() => {
		window.__FIBER_TEST__.start();
		window.__FIBER_TEST__.chunk('unauthorized');
	});
	await expect(page.getByText('unauthorized')).toBeVisible();

	await page.evaluate(() => {
		window.__FIBER_TEST__.start();
		window.__FIBER_TEST__.chunk('{"ok":true}');
	});
	await expect(page.getByText('{"ok":true}')).toBeVisible();
	await expect(page.getByText('unauthorized')).toBeHidden();
});

test.describe('the response context menu', () => {
	test('copy response and copy URL write to the clipboard', async ({ page }) => {
		await install(page, {
			sendResponse: response({
				body: '{"a":1}',
				finalUrl: 'https://api.acme.com/users'
			}),
			sections: [section({ requests: [users] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();
		await expect(page.getByRole('tab', { name: 'Pretty' })).toBeVisible();

		await page.locator('.cm-editor').nth(1).click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Copy response' }).click();
		await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('{"a":1}');

		await page.locator('.cm-editor').nth(1).click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Copy formatted' }).click();
		await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(
			JSON.stringify({ a: 1 }, null, 2)
		);

		await page.locator('.cm-editor').nth(1).click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Copy URL' }).click();
		await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(
			'https://api.acme.com/users'
		);
	});

	test("clearing this request's history empties the pane", async ({ page }) => {
		await install(page, {
			sendResponse: response({ body: '{"gone":true}' }),
			sections: [section({ requests: [users] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();
		await expect(page.getByRole('tab', { name: 'Pretty' })).toBeVisible();

		await page.locator('.cm-editor').nth(1).click({ button: 'right' });
		await page.getByRole('menuitem', { name: "Clear this request's history" }).click();
		await expect(page.getByText('Send a request to see the response.')).toBeVisible();
	});
});
