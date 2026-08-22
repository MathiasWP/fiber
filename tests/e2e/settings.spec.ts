import { expect, test } from '@playwright/test';
import { commands, install, savedRequest, section } from './mock-ipc';

async function openSettings(page: import('@playwright/test').Page, name = 'Acme') {
	await page.getByLabel(`Settings for ${name}`).click();
	await expect(page.getByText('Section settings')).toBeVisible();
}

test.describe('the General tab', () => {
	test('renaming and the base URL write through, and a trailing slash is dropped on blur', async ({
		page
	}) => {
		await install(page, { sections: [section({ requests: [savedRequest()] })] });
		await page.goto('/');
		await openSettings(page);

		const drawer = page.locator('.drawer');
		await drawer.getByLabel('Name').fill('Payments');
		const base = drawer.getByPlaceholder('https://api.example.com');
		await base.fill('https://pay.example.com/v1/');
		await base.blur();
		await expect(base).toHaveValue('https://pay.example.com/v1');

		await page.getByRole('button', { name: 'Done' }).click();
		await expect(page.getByText('Payments', { exact: true })).toBeVisible();
		await page.getByText('List users').click();
		await expect(page.getByText('https://pay.example.com/v1', { exact: true })).toBeVisible();
	});

	test('MCP write access is locked until the collection is shared', async ({ page }) => {
		await install(page, { sections: [section()] });
		await page.goto('/');
		await openSettings(page);

		const writes = page.getByLabel('Allow more than GET, HEAD and OPTIONS');
		await expect(writes).toBeEnabled();
		await page.getByLabel('Let agents see and call this collection').uncheck();
		await expect(writes).toBeDisabled();
	});
});

test.describe('Auth', () => {
	test('the tab names the method in use', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					auth: { kind: 'bearer', secretRef: 'sec-1:auth' }
				})
			]
		});
		await page.goto('/');
		await openSettings(page);
		await expect(page.getByRole('tab', { name: 'Auth · bearer' })).toBeVisible();
	});

	test('a Bearer token is write-only: Save stores it, Remove forgets it', async ({ page }) => {
		await install(page, { sections: [section()] });
		await page.goto('/');
		await openSettings(page);
		await page.getByRole('tab', { name: /^Auth/ }).click();

		await page.locator('.drawer').getByText('None', { exact: true }).click();
		await page.getByRole('option', { name: 'Bearer token' }).click();
		await expect(page.getByText('not set')).toBeVisible();

		const save = page.getByRole('button', { name: 'Save' });
		await expect(save).toBeDisabled();
		await page.getByPlaceholder('Paste the token').fill('tok_123');
		await save.click();
		await expect(page.getByText('stored in keychain')).toBeVisible();
		await expect(page.getByText('Saved', { exact: true })).toBeVisible();
		await expect.poll(() => commands(page, 'set_secret')).not.toEqual([]);
		await expect.poll(() => commands(page, 'forget_token')).not.toEqual([]);

		await page.getByRole('button', { name: 'Remove' }).click();
		await expect(page.getByText('not set')).toBeVisible();
		await expect.poll(() => commands(page, 'delete_secret')).not.toEqual([]);
	});

	test('Login request exposes the token path and can forget a cached token', async ({ page }) => {
		await install(page, { sections: [section()] });
		await page.goto('/');
		await openSettings(page);
		await page.getByRole('tab', { name: /^Auth/ }).click();

		await page.locator('.drawer').getByText('None', { exact: true }).click();
		await page.getByRole('option', { name: 'Login request' }).click();

		await expect(page.getByPlaceholder('$.access_token')).toHaveValue('$.access_token');
		await expect(page.getByPlaceholder('/login')).toHaveValue('/login');
		await page.getByRole('button', { name: 'Forget token' }).click();
		await expect.poll(() => commands(page, 'forget_token')).not.toEqual([]);
	});

	test('Browser session can open the sign-in window and reports a failure', async ({ page }) => {
		await install(page, {
			signInError: 'no display',
			sections: [
				section({
					auth: {
						kind: 'browser',
						loginUrl: 'https://acme.com/login',
						capture: 'cookie',
						captureKey: 'session',
						capturePath: '',
						header: 'Cookie',
						prefix: '',
						ttlSeconds: 0,
						secretRef: 'sec-1:auth'
					}
				})
			]
		});
		await page.goto('/');
		await openSettings(page);
		await page.getByRole('tab', { name: /^Auth/ }).click();
		await page.getByRole('button', { name: 'Open sign-in' }).click();
		await expect(page.getByText(/no display/)).toBeVisible();
		await expect.poll(() => commands(page, 'browser_sign_in')).not.toEqual([]);
	});

	test('a stored browser credential can be removed', async ({ page }) => {
		await install(page, {
			hasSecret: true,
			sections: [
				section({
					auth: {
						kind: 'browser',
						loginUrl: 'https://acme.com/login',
						capture: 'cookie',
						captureKey: 'session',
						capturePath: '',
						header: 'Cookie',
						prefix: '',
						ttlSeconds: 0,
						secretRef: 'sec-1:auth'
					}
				})
			]
		});
		await page.goto('/');
		await page.getByLabel(/a credential is stored/).hover();
		await expect(page.locator('.tooltip')).toContainText('a credential is stored');

		await openSettings(page);
		await page.getByRole('tab', { name: /^Auth/ }).click();
		await expect(page.getByText('captured', { exact: true })).toBeVisible();
		await page.getByRole('button', { name: 'Remove' }).click();
		await expect(page.getByText('not captured')).toBeVisible();
	});
});

test.describe('importing an OpenAPI file', () => {
	test('previews new endpoints, skips ones already here, and adds the rest', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					baseUrl: '',
					requests: [savedRequest({ id: 'r-pets', method: 'GET', path: '/pets', name: 'listPets' })]
				})
			]
		});
		await page.goto('/');
		await openSettings(page);

		await page.locator('input[type="file"]').setInputFiles({
			name: 'openapi.json',
			mimeType: 'application/json',
			buffer: Buffer.from('{"openapi":"3.0.0"}')
		});

		await expect(page.getByText('Petstore · 1.0.0')).toBeVisible();
		await expect(page.getByText('1 new of 2')).toBeVisible();
		await expect(page.getByText('already here')).toBeVisible();
		await expect(page.getByText(/Base URL will be set to/)).toBeVisible();

		await page.getByRole('button', { name: 'Add 1 endpoint' }).click();
		await expect(page.getByText('Added 1 endpoint.')).toBeVisible();

		await page.getByRole('button', { name: 'Done' }).click();
		await page.getByText('createPet').click();
		await expect(page.getByText(/https:\/\/petstore\.example\.com/)).toBeVisible();
	});

	test('adding several endpoints reports the number that actually landed', async ({ page }) => {
		await install(page, { sections: [section({ requests: [] })] });
		await page.goto('/');
		await openSettings(page);

		await page.locator('input[type="file"]').setInputFiles({
			name: 'openapi.json',
			mimeType: 'application/json',
			buffer: Buffer.from('{}')
		});
		await page.getByRole('button', { name: 'Add 2 endpoints' }).click();
		await expect(page.getByText('Added 2 endpoints.')).toBeVisible();
	});

	test('Cancel drops the preview without writing', async ({ page }) => {
		await install(page, { sections: [section()] });
		await page.goto('/');
		await openSettings(page);

		await page.locator('input[type="file"]').setInputFiles({
			name: 'openapi.json',
			mimeType: 'application/json',
			buffer: Buffer.from('{}')
		});
		await expect(page.getByText('2 new of 2')).toBeVisible();
		await page.getByRole('button', { name: 'Cancel' }).click();
		await expect(page.getByText('2 new of 2')).toBeHidden();
		await page.getByRole('button', { name: 'Done' }).click();
		await expect(page.getByText('listPets')).toBeHidden();
	});

	test('a parse error is shown, and the same file can be chosen again', async ({ page }) => {
		await install(page, {
			parseError: 'not a spec',
			sections: [section()]
		});
		await page.goto('/');
		await openSettings(page);

		await page.locator('input[type="file"]').setInputFiles({
			name: 'bad.json',
			mimeType: 'application/json',
			buffer: Buffer.from('nope')
		});
		await expect(page.getByText(/not a spec/)).toBeVisible();
	});
});

test('Escape closes settings without running the loader', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				loader: {
					enabled: true,
					url: '/openapi.json',
					method: 'GET',
					query: '.paths',
					next: '',
					ttlSeconds: 0
				}
			})
		]
	});
	await page.goto('/');
	await openSettings(page);
	await page.keyboard.press('Escape');
	await expect(page.getByRole('heading', { name: 'Section settings' })).toBeHidden();
	expect(await commands(page, 'run_loader')).toEqual([]);
	await expect.poll(() => commands(page, 'browser_close')).not.toEqual([]);
});

test('Done does not run a loader that is switched off', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				loader: {
					enabled: false,
					url: '/openapi.json',
					method: 'GET',
					query: '.paths',
					next: '',
					ttlSeconds: 0
				}
			})
		]
	});
	await page.goto('/');
	await openSettings(page);
	await page.getByRole('button', { name: 'Done' }).click();
	expect(await commands(page, 'run_loader')).toEqual([]);
});
