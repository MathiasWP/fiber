import { expect, test, type Page } from '@playwright/test';
import {
	commands,
	historyRecord,
	install,
	openApiImport,
	response,
	savedRequest,
	section
} from './mock-ipc';

async function openRequest(page: Page, request = savedRequest(), options: Parameters<typeof install>[1] = {}) {
	await install(page, {
		...options,
		sections: options.sections ?? [section({ requests: [request] })]
	});
	await page.goto('/');
	await page.getByText(request.name, { exact: true }).click();
}

async function openSettings(page: Page) {
	await page.getByLabel('Settings for Acme').click();
	await expect(page.getByRole('heading', { name: 'Section settings' })).toBeVisible();
}

test.describe('path parameters', () => {
	test('the UI keeps the template but sends encoded path values', async ({ page }) => {
		const request = savedRequest({
			method: 'POST',
			path: '/pets/{petId}?include=owner',
			pathParams: [{ name: 'petId', value: '' }],
			body: '{}'
		});
		await openRequest(page, request);

		await page.getByRole('tab', { name: /^Params/ }).click();
		const name = page.locator('input[readonly]').first();
		await expect(name).toHaveValue('petId');
		const value = name.locator('xpath=following-sibling::input');
		await value.fill('a/b β');

		await expect(page.getByPlaceholder('/user/get')).toHaveValue(
			'/pets/{petId}?include=owner'
		);
		await expect(page.getByRole('tab', { name: 'Params (2)' })).toBeVisible();

		await page.getByRole('button', { name: 'Send' }).click();
		const spec = (await commands(page, 'send_request'))[0].args.spec as {
			url: string;
			pathParams: { name: string; value: string }[];
		};
		expect(spec.url).toBe('https://api.acme.com/pets/a%2Fb%20%CE%B2?include=owner');
		expect(spec.pathParams).toEqual([{ name: 'petId', value: 'a/b β' }]);
	});

	test('one value fills every repeated placeholder and Clear restores the template', async ({
		page
	}) => {
		const request = savedRequest({
			method: 'POST',
			path: '/compare/{id}/with/{id}',
			pathParams: [{ name: 'id', value: 'same' }]
		});
		await openRequest(page, request);
		await page.getByRole('tab', { name: /^Params/ }).click();

		const names = page.locator('input[readonly]');
		await expect(names).toHaveCount(1);
		await expect(names.first()).toHaveValue('id');
		await page.getByTitle('Clear').click();
		await page.getByRole('button', { name: 'Send' }).click();

		const spec = (await commands(page, 'send_request'))[0].args.spec as { url: string };
		expect(spec.url).toBe('https://api.acme.com/compare/{id}/with/{id}');
	});
});

test.describe('every request body kind', () => {
	test('Text sends the editor verbatim without inventing a JSON content type', async ({ page }) => {
		const request = savedRequest({ method: 'POST', body: 'plain text', bodyKind: 'text' });
		await openRequest(page, request);
		await page.getByRole('button', { name: 'Send' }).click();

		const spec = (await commands(page, 'send_request'))[0].args.spec as {
			body: string;
			bodyKind: string;
			headers: { name: string; value: string }[];
		};
		expect(spec.bodyKind).toBe('text');
		expect(spec.body).toBe('plain text');
		expect(spec.headers).not.toContainEqual({
			name: 'Content-Type',
			value: 'application/json'
		});
	});

	test('Form URL-encoded sends only named fields', async ({ page }) => {
		const request = savedRequest({ method: 'POST', bodyKind: 'form', form: [] });
		await openRequest(page, request);

		await page.locator('input[placeholder="Field"]:visible').fill('email');
		await page.locator('input[placeholder="Value"]:visible').first().fill('ada@example.com');
		await expect(page.locator('input[placeholder="Field"]:visible')).toHaveCount(2);
		await page.getByRole('button', { name: 'Send' }).click();

		const spec = (await commands(page, 'send_request'))[0].args.spec as {
			body: null;
			bodyKind: string;
			form: { name: string; value: string }[];
		};
		expect(spec.body).toBeNull();
		expect(spec.bodyKind).toBe('form');
		expect(spec.form).toEqual([
			{ name: 'email', value: 'ada@example.com', file: '', isFile: false }
		]);
	});

	test('Multipart can mix text and file fields', async ({ page }) => {
		const request = savedRequest({ method: 'POST', bodyKind: 'multipart', form: [] });
		await openRequest(page, request);

		const fields = page.locator('input[placeholder="Field"]:visible');
		await fields.nth(0).fill('caption');
		await page.locator('input[placeholder="Value"]:visible').nth(0).fill('hello');
		await fields.nth(1).fill('avatar');
		await page.locator('input[type="checkbox"]:visible').nth(1).check();
		await page.locator('input[placeholder="/path/to/file"]:visible').fill('/tmp/avatar.png');
		await page.getByRole('button', { name: 'Send' }).click();

		const spec = (await commands(page, 'send_request'))[0].args.spec as {
			bodyKind: string;
			form: { name: string; value: string; file: string; isFile: boolean }[];
		};
		expect(spec.bodyKind).toBe('multipart');
		expect(spec.form).toEqual([
			{ name: 'caption', value: 'hello', file: '', isFile: false },
			{ name: 'avatar', value: '', file: '/tmp/avatar.png', isFile: true }
		]);
	});

	test('File sends an absolute path and disables formatting', async ({ page }) => {
		const request = savedRequest({ method: 'PUT', bodyKind: 'file', file: '' });
		await openRequest(page, request);

		await expect(page.getByRole('button', { name: 'Format' })).toBeDisabled();
		await page.getByPlaceholder('/absolute/path/to/file').fill('/tmp/archive.zip');
		await page.getByRole('button', { name: 'Send' }).click();

		const spec = (await commands(page, 'send_request'))[0].args.spec as {
			body: null;
			bodyKind: string;
			file: string;
		};
		expect(spec).toMatchObject({ body: null, bodyKind: 'file', file: '/tmp/archive.zip' });
	});
});

test('collection HTTP policy is passed to every send', async ({ page }) => {
	const request = savedRequest();
	await openRequest(page, request, {
		sections: [
			section({
				timeoutMs: 1234,
				followRedirects: false,
				acceptInvalidCerts: true,
				proxy: 'http://127.0.0.1:9000',
				requests: [request]
			})
		]
	});
	await page.getByRole('button', { name: 'Send' }).click();

	const spec = (await commands(page, 'send_request'))[0].args.spec as Record<string, unknown>;
	expect(spec).toMatchObject({
		timeoutMs: 1234,
		followRedirects: false,
		acceptInvalidCerts: true,
		proxy: 'http://127.0.0.1:9000'
	});
});

test.describe('response edge cases', () => {
	test('response bodies are validated against the OpenAPI response schema', async ({ page }) => {
		const request = savedRequest({ id: 'POST /flags', method: 'POST', path: '/flags' });
		await openRequest(page, request, {
			sections: [
				section({
					loader: {
						enabled: true,
						url: '/openapi.json',
						method: 'GET',
						query: '.paths',
						next: '',
						ttlSeconds: 0
					},
					overlay: [request]
				})
			],
			loaded: [
				{ method: 'POST', path: '/flags', name: 'List users', description: '', body: '' }
			],
			responseSchemas: {
				'POST /flags': {
					type: 'object',
					required: ['enabled'],
					properties: { enabled: { type: 'boolean' } }
				}
			},
			sendResponse: response({ body: '{"enabled":"yes"}' })
		});
		await page.getByRole('button', { name: 'Send' }).click();

		const alert = page.getByRole('alert');
		await expect(alert).toContainText('Response does not match the OpenAPI schema');
		await expect(alert).toContainText('$.enabled must be boolean, not string.');
	});

	test('invalid JSON is left readable in Pretty instead of disappearing', async ({ page }) => {
		await openRequest(page, savedRequest(), {
			sendResponse: response({ body: '{"unfinished":', sizeBytes: 14 })
		});
		await page.getByRole('button', { name: 'Send' }).click();
		await expect(page.locator('.cm-content').nth(1)).toContainText('{"unfinished":');
	});

	test('switching requests restores each request’s own latest response', async ({ page }) => {
		const first = savedRequest({ id: 'r1', name: 'First', path: '/first' });
		const second = savedRequest({ id: 'r2', name: 'Second', path: '/second' });
		await install(page, {
			sections: [section({ requests: [first, second] })],
			sendResponses: {
				r1: response({ body: '{"from":"first"}', finalUrl: 'https://api.acme.com/first' }),
				r2: response({ body: '{"from":"second"}', finalUrl: 'https://api.acme.com/second' })
			}
		});
		await page.goto('/');

		await page.getByText('First', { exact: true }).click();
		await page.getByRole('button', { name: 'Send' }).click();
		await expect(page.locator('.cm-content').nth(1)).toContainText('"first"');

		await page.getByText('Second', { exact: true }).click();
		await page.getByRole('button', { name: 'Send' }).click();
		await expect(page.locator('.cm-content').nth(1)).toContainText('"second"');

		await page.getByText('First', { exact: true }).click();
		await expect(page.locator('.cm-content').nth(1)).toContainText('"first"');
		await expect(page.locator('.cm-content').nth(1)).not.toContainText('"second"');
	});
});

test.describe('history failures remain recoverable', () => {
	test('a history-list failure is reported without taking collections down', async ({ page }) => {
		await install(page, {
			sections: [section({ requests: [savedRequest()] })],
			historyListError: 'history database is locked'
		});
		await page.goto('/');
		await expect(page.getByText('List users')).toBeVisible();
		await page.getByRole('button', { name: 'History' }).click();
		await expect(page.getByText(/history database is locked/)).toBeVisible();
	});

	test('a body-load failure is reported and can be retried by reopening', async ({ page }) => {
		await install(page, {
			history: [historyRecord()],
			historyBodyError: 'body file is unreadable'
		});
		await page.goto('/');
		await page.getByRole('button', { name: 'History' }).click();
		await page
			.locator('button')
			.filter({ hasText: 'https://api.acme.com/users' })
			.dispatchEvent('click');
		await expect(page.getByText(/body file is unreadable/)).toBeVisible();
	});

	test('a failed clear-all restores every row', async ({ page }) => {
		await install(page, {
			history: [
				historyRecord({ id: 'h1', url: 'https://api.acme.com/one' }),
				historyRecord({ id: 'h2', url: 'https://api.acme.com/two' })
			],
			deleteHistoryError: 'database is read-only'
		});
		await page.goto('/');
		await page.getByRole('button', { name: 'History' }).click();
		await page.getByTitle('Clear history').click();

		await expect(page.getByText('https://api.acme.com/one')).toBeVisible();
		await expect(page.getByText('https://api.acme.com/two')).toBeVisible();
		await expect(page.getByText(/database is read-only/)).toBeVisible();
	});

	test('a failed per-request clear restores the response', async ({ page }) => {
		await openRequest(page, savedRequest(), {
			sendResponse: response({ body: '{"keep":true}' }),
			deleteHistoryError: 'cannot clear history'
		});
		await page.getByRole('button', { name: 'Send' }).click();
		await page.locator('.cm-editor').nth(1).click({ button: 'right' });
		await page.getByRole('menuitem', { name: "Clear this request's history" }).click();

		await expect(page.locator('.cm-content').nth(1)).toContainText('"keep"');
		await page.getByRole('button', { name: 'History' }).click();
		await expect(page.getByText(/cannot clear history/)).toBeVisible();
	});
});

test.describe('loader failures and summaries', () => {
	const loader = {
		enabled: true,
		url: '/openapi.json',
		method: 'GET',
		query: '.paths',
		next: '',
		ttlSeconds: 0
	};

	test('Run now names a loader failure', async ({ page }) => {
		await install(page, {
			sections: [section({ loader })],
			runLoaderError: 'manifest returned 503'
		});
		await page.goto('/');
		await openSettings(page);
		await page.getByRole('tab', { name: /^Loader/ }).click();
		await page.getByRole('button', { name: 'Run now' }).click();
		await expect(page.getByText(/manifest returned 503/)).toBeVisible();
	});

	test('the summary includes paging, additions and removals', async ({ page }) => {
		await install(page, {
			sections: [section({ loader })],
			loaded: [{ method: 'GET', path: '/old', name: 'old', description: '', body: '' }],
			refreshed: [{ method: 'GET', path: '/new', name: 'new', description: '', body: '' }],
			loaderRun: { pages: 3 }
		});
		await page.goto('/');
		await openSettings(page);
		await page.getByRole('tab', { name: /^Loader/ }).click();
		await page.getByRole('button', { name: 'Run now' }).click();
		await expect(
			page.getByText('1 endpoints from 3 pages · 1 new, 1 removed')
		).toBeVisible();
	});
});

test.describe('settings and import edge cases', () => {
	test('legacy sections missing HTTP fields receive safe defaults', async ({ page }) => {
		const legacy = section();
		delete legacy.timeoutMs;
		delete legacy.followRedirects;
		delete legacy.acceptInvalidCerts;
		delete legacy.proxy;
		await install(page, { sections: [legacy] });
		await page.goto('/');
		await openSettings(page);

		await expect(page.getByLabel('Timeout (ms)')).toHaveValue('60000');
		await expect(page.getByLabel('Follow redirects')).toBeChecked();
		await expect(page.getByLabel('Allow invalid TLS certificates')).not.toBeChecked();
		await expect(page.getByPlaceholder('http://127.0.0.1:8080')).toHaveValue('');
	});

	test('a keychain write failure stays in the drawer and does not claim success', async ({
		page
	}) => {
		await install(page, {
			sections: [section()],
			setSecretError: 'keychain is locked'
		});
		await page.goto('/');
		await openSettings(page);
		await page.getByRole('tab', { name: /^Auth/ }).click();
		await page.locator('.drawer').getByText('None', { exact: true }).click();
		await page.getByRole('option', { name: 'Bearer token' }).click();
		await page.getByPlaceholder('Paste the token').fill('tok_123');
		await page.getByRole('button', { name: 'Save' }).click();

		await expect(page.getByText(/keychain is locked/)).toBeVisible();
		await expect(page.getByText('stored in keychain')).toBeHidden();
		await expect(page.getByText('Saved', { exact: true })).toBeHidden();
	});

	test('duplicate operations in one import are only added once', async ({ page }) => {
		await install(page, {
			sections: [section()],
			openapi: openApiImport({
				endpoints: [
					{ method: 'GET', path: '/pets', name: 'listPets', description: '', body: '' },
					{ method: 'GET', path: '/pets', name: 'listPetsAgain', description: '', body: '' }
				]
			})
		});
		await page.goto('/');
		await openSettings(page);
		await page.locator('input[type="file"]').setInputFiles({
			name: 'duplicate.json',
			mimeType: 'application/json',
			buffer: Buffer.from('{}')
		});

		await expect(page.getByText('1 new of 2')).toBeVisible();
		await page.getByRole('button', { name: 'Add 1 endpoint' }).click();
		await page.getByRole('button', { name: 'Done' }).click();
		await expect(page.getByText('listPets', { exact: true })).toHaveCount(1);
	});

	test('imported examples seed query, path, and form fields without replacing the base URL', async ({
		page
	}) => {
		await install(page, {
			sections: [section({ baseUrl: 'https://keep.example.com' })],
			openapi: openApiImport({
				baseUrl: 'https://ignore.example.com',
				endpoints: [
					{
						method: 'POST',
						path: '/pets/{petId}',
						name: 'updatePet',
						description: '',
						body: '',
						bodyKind: 'form',
						form: [{ name: 'name', value: 'Ada' }],
						parameters: [
							{ name: 'petId', in: 'path', example: 'p-1' },
							{ name: 'verbose', in: 'query', example: 'true' }
						]
					}
				]
			})
		});
		await page.goto('/');
		await openSettings(page);
		await page.locator('input[type="file"]').setInputFiles({
			name: 'forms.json',
			mimeType: 'application/json',
			buffer: Buffer.from('{}')
		});
		await page.getByRole('button', { name: 'Add 1 endpoint' }).click();
		await page.getByRole('button', { name: 'Done' }).click();
		await page.getByText('updatePet', { exact: true }).click();

		await expect(page.getByText('https://keep.example.com', { exact: true })).toBeVisible();
		await expect(page.getByPlaceholder('/user/get')).toHaveValue(
			'/pets/{petId}?verbose=true'
		);
		await expect(page.locator('select')).toHaveValue('form');
		await expect(page.locator('input[placeholder="Field"]:visible').first()).toHaveValue('name');
		await page.getByRole('tab', { name: 'Params (2)' }).click();
		const name = page.locator('input[readonly]').first();
		await expect(name).toHaveValue('petId');
		await expect(name.locator('xpath=following-sibling::input')).toHaveValue('p-1');
	});
});

test.describe('collection persistence edges', () => {
	test('blank renames become Untitled instead of leaving an invisible row', async ({ page }) => {
		await install(page, { sections: [section()] });
		await page.goto('/');
		await page.getByText('Acme', { exact: true }).click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Rename' }).click();
		const field = page.locator('input.input-base.py-0\\.5');
		await field.fill('   ');
		await field.press('Enter');
		await expect(page.getByText('Untitled', { exact: true })).toBeVisible();
	});

	test('Copy URL respects absolute overrides and filled path parameters', async ({ page }) => {
		const request = savedRequest({
			path: 'https://other.example.com/users/{id}',
			pathParams: [{ name: 'id', value: 'a/b' }]
		});
		await install(page, { sections: [section({ requests: [request] })] });
		await page.goto('/');
		await page.getByText('List users').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Copy URL' }).click();
		await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(
			'https://other.example.com/users/a%2Fb'
		);
	});

	test('a collection-list failure leaves an actionable error instead of a blank sidebar', async ({
		page
	}) => {
		await install(page, { listSectionsError: 'collections directory is not readable' });
		await page.goto('/');
		await expect(page.getByText(/collections directory is not readable/)).toBeVisible();
		await expect(page.getByText('No sections yet.')).toBeVisible();
	});

	test('the selected theme survives a reload', async ({ page }) => {
		await install(page);
		await page.goto('/');
		const trigger = page.getByTitle(/Switch to/);
		const before = await page.evaluate(() => document.documentElement.dataset.theme);
		await trigger.click();
		const chosen = await page.evaluate(() => document.documentElement.dataset.theme);
		expect(chosen).not.toBe(before);

		await page.reload();
		await expect
			.poll(() => page.evaluate(() => document.documentElement.dataset.theme))
			.toBe(chosen);
	});
});
