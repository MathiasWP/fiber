import { expect, test } from '@playwright/test';
import { commands, install, section } from './mock-ipc';

async function openPicker(page: import('@playwright/test').Page) {
	await page.getByLabel('Settings for Acme').click();
	await page.getByRole('tab', { name: /^Auth/ }).click();
	await page.getByRole('button', { name: 'Pick credential…' }).click();
	await expect(page.getByText('Pick your credential')).toBeVisible();
}

const browserAuth = {
	kind: 'browser' as const,
	loginUrl: 'https://acme.com/login',
	capture: 'cookie' as const,
	captureKey: 'session',
	capturePath: '',
	header: 'Cookie',
	prefix: '',
	ttlSeconds: 0,
	secretRef: 'sec-1:auth'
};

test('picking a cookie stores it and names the capture rule', async ({ page }) => {
	await install(page, { sections: [section({ auth: browserAuth })] });
	await page.goto('/');
	await openPicker(page);

	await expect(page.getByText('Auth0', { exact: true })).toBeVisible();
	await page.getByText('session', { exact: true }).click();

	await expect(page.getByText('Pick your credential')).toBeHidden();
	await expect(page.getByText('captured', { exact: true })).toBeVisible();
	await expect(page.locator('p.font-mono')).toHaveText(/cookie/);
	await expect(page.locator('p.font-mono')).toHaveText(/session/);
	await expect.poll(() => commands(page, 'browser_capture')).not.toEqual([]);
});

test('picking a storage key stores it as a storage rule', async ({ page }) => {
	await install(page, {
		snapshot: {
			localStorage: [{ key: 'access_token', value: 'tok_from_storage' }],
			cookies: [],
			indexedDb: []
		},
		sections: [section({ auth: browserAuth })]
	});
	await page.goto('/');
	await openPicker(page);

	await page.getByText('access_token', { exact: true }).click();
	await expect(page.getByText('captured', { exact: true })).toBeVisible();
	await expect(page.locator('p.font-mono')).toHaveText(/storage/);
	await expect(page.locator('p.font-mono')).toHaveText(/access_token/);
});

test('the filter is a substring match, and an empty result says how many were hidden', async ({
	page
}) => {
	await install(page, { sections: [section({ auth: browserAuth })] });
	await page.goto('/');
	await openPicker(page);

	const filter = page.getByPlaceholder('Filter by name, path or value…');
	await filter.fill('no-such-value');
	await expect(page.getByText(/Nothing matches “no-such-value”/)).toBeVisible();
	await expect(page.getByText(/values in this session/)).toBeVisible();
});

test('an empty session explains what to do next', async ({ page }) => {
	await install(page, {
		snapshot: { localStorage: [], cookies: [], indexedDb: [] },
		sections: [section({ auth: browserAuth })]
	});
	await page.goto('/');
	await openPicker(page);
	await expect(page.getByText(/Nothing found/)).toBeVisible();
});

test('a snapshot error can be retried', async ({ page }) => {
	await install(page, {
		snapshotError: 'window closed',
		sections: [section({ auth: browserAuth })]
	});
	await page.goto('/');
	await openPicker(page);
	await expect(page.getByText(/window closed/)).toBeVisible();
	await page.getByRole('button', { name: 'Try again' }).click();
	await expect.poll(async () => (await commands(page, 'browser_snapshot')).length).toBe(2);
});

test('IndexedDB entries are listed alongside cookies', async ({ page }) => {
	await install(page, {
		snapshot: {
			localStorage: [],
			cookies: [],
			indexedDb: [
				{
					database: 'firebaseLocalStorageDb',
					store: 'firebaseLocalStorage',
					key: 'firebase:authUser:abc',
					value: '{"stsTokenManager":{"accessToken":"tok"}}'
				}
			]
		},
		sections: [section({ auth: browserAuth })]
	});
	await page.goto('/');
	await openPicker(page);
	await expect(page.getByText('Firebase', { exact: true })).toBeVisible();
	await expect(page.getByText('indexeddb')).toBeVisible();
});

test('a capture failure is shown on the auth tab', async ({ page }) => {
	await install(page, {
		captureError: 'nothing matched',
		sections: [section({ auth: browserAuth })]
	});
	await page.goto('/');
	await openPicker(page);
	await page.getByText('session', { exact: true }).click();
	await expect(page.getByText(/nothing matched/)).toBeVisible();
	await expect(page.getByText('captured', { exact: true })).toBeHidden();
});
