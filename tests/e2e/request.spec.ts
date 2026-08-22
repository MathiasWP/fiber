import { expect, test } from '@playwright/test';
import { commands, install, response, savedRequest, section } from './mock-ipc';

const users = savedRequest();

test('Send stays disabled until there is a URL to hit', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await expect(page.getByRole('button', { name: 'Send' })).toBeDisabled();
});

test('a scratch URL can be sent without saving a request first', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await page.getByPlaceholder('https://api.example.com/users').fill('https://example.com/ping');
	await page.getByRole('button', { name: 'Send' }).click();
	await expect(page.getByText('200 OK')).toBeVisible();

	const sent = await commands(page, 'send_request');
	expect(sent).toHaveLength(1);
	expect((sent[0].args.spec as { url: string }).url).toBe('https://example.com/ping');
});

test('a collection request shows the base URL beside the path', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	await page.goto('/');

	await page.getByText('List users').click();
	await expect(page.getByText('https://api.acme.com', { exact: true })).toBeVisible();
	await expect(page.getByPlaceholder('/user/get')).toHaveValue('/users');
});

test('an unnamed request follows the path until someone names it', async ({ page }) => {
	await install(page, {
		sections: [section({ requests: [savedRequest({ name: 'New request', path: '/' })] })]
	});
	await page.goto('/');

	await page.getByText('New request').click();
	await page.getByPlaceholder('/user/get').fill('/orders');
	await expect(page.getByText('/orders', { exact: true })).toBeVisible();
});

test.describe('the method picker', () => {
	test('GET and HEAD show Params instead of Body', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');
		await page.getByText('List users').click();

		await expect(page.getByRole('tab', { name: /^Params/ })).toBeVisible();
		await expect(page.getByRole('tab', { name: 'Body' })).toBeHidden();

		await page.getByLabel('HTTP method').click();
		await page.getByRole('option', { name: 'HEAD' }).click();
		await expect(page.getByRole('tab', { name: /^Params/ })).toBeVisible();
	});

	test('POST restores the Body tab and keeps Params', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');
		await page.getByText('List users').click();

		await page.getByLabel('HTTP method').click();
		await page.getByRole('option', { name: 'POST' }).click();
		await expect(page.getByRole('tab', { name: 'Body' })).toBeVisible();
		await expect(page.getByRole('tab', { name: /^Params/ })).toBeVisible();
	});
});

test.describe('query params', () => {
	test('editing a row rewrites the URL, and the URL rewrites the table', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');
		await page.getByText('List users').click();

		await page.getByPlaceholder('Parameter').first().fill('limit');
		await page.getByPlaceholder('Value').first().fill('10');
		await expect(page.getByPlaceholder('/user/get')).toHaveValue('/users?limit=10');
		await expect(page.getByRole('tab', { name: 'Params (1)' })).toBeVisible();

		await page.getByPlaceholder('/user/get').fill('/users?q=hello world');
		await expect(page.getByPlaceholder('Parameter').first()).toHaveValue('q');
		await expect(page.getByPlaceholder('Value').first()).toHaveValue('hello world');
	});

	test('values are encoded on the way into the URL', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');
		await page.getByText('List users').click();

		await page.getByPlaceholder('Parameter').first().fill('q');
		await page.getByPlaceholder('Value').first().fill('a b');
		await expect(page.getByPlaceholder('/user/get')).toHaveValue('/users?q=a+b');
	});

	test('a single empty row has no delete button', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');
		await page.getByText('List users').click();

		await expect(page.getByTitle('Remove parameter')).toBeHidden();
		await expect(page.getByTitle('Clear')).toBeHidden();
	});

	test('the X removes a filled row and clears the last one', async ({ page }) => {
		await install(page, {
			sections: [section({ requests: [savedRequest({ path: '/users?a=1&b=2' })] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();

		await page.getByTitle('Remove parameter').first().click();
		await expect(page.getByPlaceholder('/user/get')).toHaveValue('/users?b=2');

		await page.getByTitle('Remove parameter').click();
		await expect(page.getByPlaceholder('/user/get')).toHaveValue('/users');
		await expect(page.getByTitle('Remove parameter')).toBeHidden();
	});
});

test.describe('headers', () => {
	test('a filled row is counted on the tab, and the X removes it', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');
		await page.getByText('List users').click();

		await page.getByRole('tab', { name: 'Headers' }).click();
		await expect(page.locator('input[placeholder="Header"]:visible')).toBeVisible();
		await expect(page.getByTitle('Remove header')).toBeHidden();

		await page.locator('input[placeholder="Header"]:visible').fill('X-Debug');
		await page.locator('input[placeholder="Value"]:visible').first().fill('1');
		await expect(page.getByRole('tab', { name: 'Headers (1)' })).toBeVisible();
		await expect(page.locator('input[placeholder="Header"]:visible')).toHaveCount(2);

		await page.getByTitle('Remove header').click();
		await expect(page.getByRole('tab', { name: 'Headers', exact: true })).toBeVisible();
	});

	test('blank header rows never reach the file', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');
		await page.getByText('List users').click();

		await page.getByRole('tab', { name: 'Headers' }).click();
		await page.locator('input[placeholder="Header"]:visible').fill('X-Debug');
		await page.locator('input[placeholder="Value"]:visible').first().fill('1');

		await expect
			.poll(async () => {
				const saved = await page.evaluate(() => window.__FIBER_TEST__.lastSaved);
				const request = (saved as { requests?: { headers: { name: string }[] }[] } | null)
					?.requests?.[0];
				return request?.headers.map((header) => header.name) ?? null;
			})
			.toEqual(['X-Debug']);
	});
});

test('Format pretty-prints JSON and leaves invalid JSON alone', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				requests: [savedRequest({ method: 'POST', path: '/users', body: '{"a":1}' })]
			})
		]
	});
	await page.goto('/');
	await page.getByText('List users').click();

	await page.getByRole('button', { name: 'Format' }).click();
	await expect(page.locator('.cm-content').first()).toContainText('"a": 1');

	const editor = page.locator('.cm-content').first();
	await editor.click();
	await page.keyboard.press('ControlOrMeta+a');
	await page.keyboard.type('{not json');
	await page.getByRole('button', { name: 'Format' }).click();
	await expect(editor).toContainText('{not json');
});

test.describe('sending', () => {
	test('⌘Enter sends, and a JSON body gets Content-Type for free', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({
							method: 'POST',
							path: '/users',
							body: '{"ok":true}'
						})
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('List users').click();

		await page.keyboard.press('ControlOrMeta+Enter');
		await expect(page.getByRole('tab', { name: 'Pretty' })).toBeVisible();

		const sent = await commands(page, 'send_request');
		expect(sent).toHaveLength(1);
		const spec = sent[0].args.spec as {
			method: string;
			url: string;
			headers: { name: string; value: string }[];
			body: string | null;
		};
		expect(spec.method).toBe('POST');
		expect(spec.url).toBe('https://api.acme.com/users');
		expect(spec.body).toBe('{"ok":true}');
		expect(spec.headers).toContainEqual({ name: 'Content-Type', value: 'application/json' });
	});

	test('a Content-Type the user set is left alone', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({
							method: 'POST',
							path: '/users',
							body: 'hello',
							headers: [{ name: 'Content-Type', value: 'text/plain' }]
						})
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();

		const spec = (await commands(page, 'send_request'))[0].args.spec as {
			headers: { name: string; value: string }[];
		};
		expect(spec.headers.filter((header) => header.name.toLowerCase() === 'content-type')).toEqual([
			{ name: 'Content-Type', value: 'text/plain' }
		]);
	});

	test('GET never sends a body, even if one is sitting in the editor', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [savedRequest({ method: 'GET', path: '/users', body: '{"no":true}' })]
				})
			]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();

		const spec = (await commands(page, 'send_request'))[0].args.spec as { body: string | null };
		expect(spec.body).toBeNull();
	});

	test('Cancel aborts an in-flight request', async ({ page }) => {
		await install(page, {
			deferSend: true,
			sections: [section({ requests: [users] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();

		await expect(page.getByRole('button', { name: 'Cancel' })).toBeVisible();
		await page.getByRole('button', { name: 'Cancel' }).click();
		await expect.poll(() => commands(page, 'cancel_request')).not.toEqual([]);
		await expect(page.getByText('Request failed')).toBeVisible();
		await expect(page.getByText(/cancelled/)).toBeVisible();
		await expect(page.getByRole('button', { name: 'Send' })).toBeVisible();
	});

	test('a network error is shown rather than looking like nothing happened', async ({ page }) => {
		await install(page, {
			sendError: 'connection refused',
			sections: [section({ requests: [users] })]
		});
		await page.goto('/');
		await page.getByText('List users').click();
		await page.getByRole('button', { name: 'Send' }).click();
		await expect(page.getByText('Request failed')).toBeVisible();
		await expect(page.getByText(/connection refused/)).toBeVisible();
	});
});

test('⌘+ grows the editor that last had focus', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				requests: [savedRequest({ method: 'POST', path: '/users', body: '{"a":1}' })]
			})
		]
	});
	await page.goto('/');
	await page.getByText('List users').click();

	const editor = page.locator('.cm-editor').first();
	await editor.click();
	const before = await editor.evaluate((node) => getComputedStyle(node).fontSize);
	await page.keyboard.press('ControlOrMeta+=');
	await expect.poll(() => editor.evaluate((node) => getComputedStyle(node).fontSize)).not.toBe(
		before
	);

	await page.keyboard.press('ControlOrMeta+0');
	await expect.poll(() => editor.evaluate((node) => getComputedStyle(node).fontSize)).toBe(before);
});
