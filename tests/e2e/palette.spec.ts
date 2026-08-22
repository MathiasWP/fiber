import { expect, test, type Page } from '@playwright/test';
import { install, savedRequest, section } from './mock-ipc';

/**
 * Chromium swallows ⌘K/Ctrl+K for its own find-in-page, so the keystroke
 * Playwright would send never reaches the app. The mock backend exposes the
 * same open path the shortcut would take.
 */
async function openPalette(page: Page) {
	await page.waitForFunction(
		() => !window.__FIBER_TEST__.openPalette.toString().includes('has not registered')
	);
	await page.evaluate(() => window.__FIBER_TEST__.openPalette());
	return page.getByRole('dialog', { name: 'Search endpoints' });
}

test('⌘K searches every collection and Enter picks the highlighted row', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				requests: [
					savedRequest(),
					savedRequest({ id: 'r2', name: 'Create user', method: 'POST', path: '/users' })
				]
			}),
			section({
				id: 'sec-2',
				name: 'Billing',
				order: 1,
				requests: [savedRequest({ id: 'r3', name: 'Invoices', method: 'GET', path: '/invoices' })]
			})
		]
	});
	await page.goto('/');

	const palette = await openPalette(page);
	await expect(palette.getByText('Acme', { exact: true })).toBeVisible();
	await expect(palette.getByText('Billing')).toBeVisible();

	const search = palette.getByPlaceholder('Search endpoints…');
	await search.fill('invo');
	await expect(palette.getByText('Invoices', { exact: true })).toBeVisible();
	await expect(palette.getByText('List users')).toBeHidden();
	await search.press('Enter');

	await expect(page.getByPlaceholder('/user/get')).toHaveValue('/invoices');
});

test('arrow keys move the highlight, and a click selects', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				requests: [
					savedRequest({ id: 'r1', name: 'Alpha', path: '/a' }),
					savedRequest({ id: 'r2', name: 'Beta', path: '/b' })
				]
			})
		]
	});
	await page.goto('/');
	const palette = await openPalette(page);
	const search = palette.getByPlaceholder('Search endpoints…');
	await search.press('ArrowDown');
	await search.press('Enter');
	await expect(page.getByPlaceholder('/user/get')).toHaveValue('/b');
});

test('nothing matching is said plainly', async ({ page }) => {
	await install(page, { sections: [section({ requests: [savedRequest()] })] });
	await page.goto('/');
	const palette = await openPalette(page);
	await palette.getByPlaceholder('Search endpoints…').fill('zzzz');
	await expect(palette.getByText('Nothing matches.')).toBeVisible();
});

test('an empty workspace explains that there is nothing to search', async ({ page }) => {
	await install(page);
	await page.goto('/');
	const palette = await openPalette(page);
	await expect(palette.getByText(/No saved requests yet/)).toBeVisible();
});

test('a loaded endpoint can be opened from the palette', async ({ page }) => {
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
		],
		loaded: [
			{
				method: 'POST',
				path: '/charged',
				name: 'chargeCard',
				description: '',
				body: '{"amount": number}'
			}
		]
	});
	await page.goto('/');
	const palette = await openPalette(page);
	await palette.getByPlaceholder('Search endpoints…').fill('charge');
	await palette.getByPlaceholder('Search endpoints…').press('Enter');
	await expect(page.locator('.cm-content').first()).toContainText('"amount"');
});

test('the palette opens in front of the settings drawer', async ({ page }) => {
	await install(page, { sections: [section({ requests: [savedRequest()] })] });
	await page.goto('/');
	await page.getByLabel('Settings for Acme').click();
	const palette = await openPalette(page);
	const search = palette.getByPlaceholder('Search endpoints…');
	await search.fill('users');
	await expect(search).toHaveValue('users');
});

test('Escape dismisses the palette', async ({ page }) => {
	await install(page, { sections: [section({ requests: [savedRequest()] })] });
	await page.goto('/');
	const palette = await openPalette(page);
	await expect(palette.getByPlaceholder('Search endpoints…')).toBeVisible();
	await page.keyboard.press('Escape');
	await expect(palette).toBeHidden();
});
