import { expect, test } from '@playwright/test';
import { commands, install, savedRequest, section } from './mock-ipc';

test('an empty sidebar explains how to start', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await expect(page.getByText('No sections yet.')).toBeVisible();
	await expect(page.getByText('v0.0.0-test')).toBeVisible();
});

test.describe('creating a collection', () => {
	test('the name field is focused and Enter submits', async ({ page }) => {
		await install(page);
		await page.goto('/');

		await page.locator('button', { has: page.locator('.i-lucide-folder-plus') }).first().click();
		await expect(page.getByText('New collection')).toBeVisible();

		const name = page.getByPlaceholder('Payments API');
		await expect(name).toBeFocused();
		await name.fill('Payments');
		await page.getByPlaceholder('https://api.example.com', { exact: true }).fill(
			'https://pay.example.com/'
		);
		await name.press('Enter');

		await expect(page.getByText('Payments', { exact: true })).toBeVisible();
		// A new collection always starts with a request to type into.
		await expect(page.getByText('New request')).toBeVisible();
		await expect(page.getByPlaceholder('/user/get')).toHaveValue('/');
		// Submitting with Enter never blurs, so the slash has to be stripped here too.
		// The chip is only on screen once that request is selected.
		await expect(page.getByText('https://pay.example.com', { exact: true })).toBeVisible();
	});

	test('empty fields read as "changed my mind" rather than untitled', async ({ page }) => {
		await install(page);
		await page.goto('/');

		await page.locator('button', { has: page.locator('.i-lucide-folder-plus') }).first().click();
		await page.getByRole('button', { name: 'Create' }).click();
		await expect(page.getByText('New collection')).toBeHidden();
		await expect(page.getByText('No sections yet.')).toBeVisible();
	});

	test('Cancel and the overlay both dismiss without creating', async ({ page }) => {
		await install(page);
		await page.goto('/');

		await page.locator('button', { has: page.locator('.i-lucide-folder-plus') }).first().click();
		await page.getByPlaceholder('Payments API').fill('Nope');
		await page.getByRole('button', { name: 'Cancel' }).click();
		await expect(page.getByText('Nope')).toBeHidden();

		await page.locator('button', { has: page.locator('.i-lucide-folder-plus') }).first().click();
		await page.locator('.dialog-scrim').click({ position: { x: 2, y: 2 } });
		await expect(page.getByText('New collection')).toBeHidden();
	});

	test('a name-only collection is Untitled no longer — it uses the name', async ({ page }) => {
		await install(page);
		await page.goto('/');

		await page.locator('button', { has: page.locator('.i-lucide-folder-plus') }).first().click();
		await page.getByPlaceholder('Payments API').fill('Just a name');
		await page.getByRole('button', { name: 'Create' }).click();
		await expect(page.getByText('Just a name', { exact: true })).toBeVisible();
		await expect(page.getByText('New request')).toBeVisible();
	});
});

test('a loose request sits above the collections and takes a full URL', async ({ page }) => {
	await install(page, { sections: [section()] });
	await page.goto('/');

	await page.locator('button', { has: page.locator('.i-lucide-square-plus') }).first().click();
	await expect(page.getByText('New request')).toBeVisible();
	await expect(page.getByPlaceholder('https://api.example.com/users')).toBeVisible();
	// No collection chip — there is no base URL to hang a path off.
	await expect(page.getByText('https://api.acme.com', { exact: true })).toBeHidden();
});

test.describe('the request context menu', () => {
	const users = savedRequest();

	test('rename stops the name following the path', async ({ page }) => {
		await install(page, {
			sections: [section({ requests: [savedRequest({ name: 'New request' })] })]
		});
		await page.goto('/');

		await page.getByText('New request').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Rename' }).click();
		const rename = page.locator('input.input-base.py-0\\.5');
		await rename.fill('Orders');
		await rename.press('Enter');
		await expect(page.getByText('Orders', { exact: true })).toBeVisible();

		await page.getByText('Orders', { exact: true }).click();
		await page.getByPlaceholder('/user/get').fill('/v2/orders');
		await expect(page.getByText('Orders', { exact: true })).toBeVisible();
		await expect(page.getByText('/v2/orders', { exact: true })).toBeHidden();
	});

	test('duplicate copies the request and selects the copy', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');

		await page.getByText('List users').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Duplicate' }).click();
		await expect(page.getByText('List users copy')).toBeVisible();
		await expect(page.getByPlaceholder('/user/get')).toHaveValue('/users');
	});

	test('delete removes the request without asking', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');

		await page.getByText('List users').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Delete request' }).click();
		await expect(page.getByText('List users')).toBeHidden();
		await expect.poll(() => commands(page, 'delete_section')).toEqual([]);
		await expect.poll(async () => (await commands(page, 'save_section')).length).toBeGreaterThan(0);
	});

	test('copy URL writes the joined address', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');

		await page.getByText('List users').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Copy URL' }).click();
		await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(
			'https://api.acme.com/users'
		);
	});

	test('move to another collection takes the request with it', async ({ page }) => {
		await install(page, {
			sections: [
				section({ requests: [users] }),
				section({ id: 'sec-2', name: 'Other', order: 1, requests: [] })
			]
		});
		await page.goto('/');

		await page.getByText('List users').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Move to' }).hover();
		await page.getByRole('menuitem', { name: 'Other' }).click();

		await expect(page.getByLabel('0 endpoints')).toBeVisible();
		await expect(page.getByLabel('1 endpoint')).toBeVisible();
		await expect(page.getByText('No requests yet.')).toBeVisible();
		await expect(page.getByText('List users')).toBeVisible();
	});

	test('move to no collection makes it a loose request', async ({ page }) => {
		await install(page, { sections: [section({ requests: [users] })] });
		await page.goto('/');

		await page.getByText('List users').click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Move to' }).hover();
		await page.getByRole('menuitem', { name: 'No collection' }).click();

		await expect(page.getByPlaceholder('https://api.example.com/users')).toBeVisible();
	});
});

test.describe('the collection context menu', () => {
	test('new request lands inside it', async ({ page }) => {
		await install(page, { sections: [section()] });
		await page.goto('/');

		await page.getByText('Acme', { exact: true }).click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'New request' }).click();
		// The sidebar row, named by its path rather than the ambiguous "New
		// request" text — the closing context menu can still hold a menuitem
		// with that same name mid-animation.
		await expect(page.getByTitle('/', { exact: true })).toHaveText('New request');
		await expect(page.getByPlaceholder('/user/get')).toHaveValue('/');
	});

	test('rename writes the new name', async ({ page }) => {
		await install(page, { sections: [section()] });
		await page.goto('/');

		await page.getByText('Acme', { exact: true }).click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Rename' }).click();
		const field = page.locator('input.input-base.py-0\\.5');
		await field.fill('Renamed');
		await field.press('Enter');
		await expect(page.getByText('Renamed', { exact: true })).toBeVisible();
	});

	test('delete asks first, and Cancel leaves the file alone', async ({ page }) => {
		await install(page, {
			sections: [section({ requests: [savedRequest()] })]
		});
		await page.goto('/');

		await page.getByText('Acme', { exact: true }).click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Delete section' }).click();
		const dialog = page.getByRole('dialog', { name: 'Delete section' });
		await expect(dialog).toBeVisible();
		await expect(dialog.getByText(/and its 1 request will be removed/)).toBeVisible();

		await dialog.getByRole('button', { name: 'Cancel' }).click();
		await expect(page.getByText('Acme', { exact: true })).toBeVisible();
		expect(await commands(page, 'delete_section')).toEqual([]);
	});

	test('confirming delete removes it from the sidebar', async ({ page }) => {
		await install(page, { sections: [section({ requests: [savedRequest()] })] });
		await page.goto('/');

		await page.getByText('Acme', { exact: true }).click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Delete section' }).click();
		await page.getByRole('dialog', { name: 'Delete section' }).getByRole('button', { name: 'Delete' }).click();
		await expect(page.getByText('Acme', { exact: true })).toBeHidden();
		await expect.poll(() => commands(page, 'delete_section')).not.toEqual([]);
	});

	test('a failed delete puts the collection back', async ({ page }) => {
		await install(page, {
			sections: [section({ requests: [savedRequest()] })],
			deleteSectionError: 'disk is full'
		});
		await page.goto('/');

		await page.getByText('Acme', { exact: true }).click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Delete section' }).click();
		await page.getByRole('dialog', { name: 'Delete section' }).getByRole('button', { name: 'Delete' }).click();
		await expect(page.getByText('Acme', { exact: true })).toBeVisible();
		await expect(page.getByText('List users')).toBeVisible();
		await expect(page.getByText(/disk is full/)).toBeVisible();
	});
});

test('collapsing a collection hides its requests and is saved', async ({ page }) => {
	await install(page, { sections: [section({ requests: [savedRequest()] })] });
	await page.goto('/');

	await expect(page.getByText('List users')).toBeVisible();
	await page.getByText('Acme', { exact: true }).click();
	await expect(page.getByText('List users')).toBeHidden();

	await expect
		.poll(async () => {
			const saved = await page.evaluate(() => window.__FIBER_TEST__.lastSaved);
			return (saved as { collapsed?: boolean } | null)?.collapsed ?? false;
		})
		.toBe(true);
});

test('a collection with nothing in it says so', async ({ page }) => {
	await install(page, { sections: [section()] });
	await page.goto('/');
	await expect(page.getByText('No requests yet.')).toBeVisible();
});

test('a loader-backed collection with an empty cache says so', async ({ page }) => {
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
	await expect(page.getByText('No endpoints loaded yet.')).toBeVisible();
});

test('searching with no match names the query', async ({ page }) => {
	await install(page, { sections: [section({ requests: [savedRequest()] })] });
	await page.goto('/');

	await page.getByPlaceholder('Search endpoints…').fill('zzzz');
	await expect(page.getByText('Nothing matches “zzzz”.')).toBeVisible();
});

test.describe('the footer', () => {
	test('clicking the theme flips it, and right-click follows the system again', async ({
		page
	}) => {
		await install(page);
		await page.goto('/');

		const trigger = page.getByTitle(/Switch to/);
		const before = await page.evaluate(() => document.documentElement.dataset.theme);
		await trigger.click();
		await expect
			.poll(() => page.evaluate(() => document.documentElement.dataset.theme))
			.not.toBe(before);

		await trigger.click({ button: 'right' });
		await page.getByRole('menuitem', { name: 'Follow the system' }).click();
	});

	test('the two add-buttons have tooltips that say what they do', async ({ page }) => {
		await install(page);
		await page.goto('/');

		await page.locator('button', { has: page.locator('.i-lucide-square-plus') }).first().hover();
		await expect(page.locator('.tooltip')).toContainText('New request');

		await page.locator('button', { has: page.locator('.i-lucide-folder-plus') }).first().hover();
		await expect(page.locator('.tooltip')).toContainText('New collection');
	});
});

test('a successful save does not dismiss the corrupt-file warning', async ({ page }) => {
	await install(page, {
		sections: [section({ requests: [savedRequest()] })],
		sectionErrors: [{ file: 'books.toml', message: 'expected `=` at line 3' }]
	});
	await page.goto('/');

	await expect(page.getByText(/Skipped 1 collection file/)).toBeVisible();
	await page.getByText('List users').click();
	await page.getByPlaceholder('/user/get').fill('/users?limit=1');

	await expect.poll(async () => (await commands(page, 'save_section')).length).toBeGreaterThan(0);
	await expect(page.getByText(/Skipped 1 collection file/)).toBeVisible();
	await expect(page.getByText(/untouched on disk/)).toBeVisible();
});

test('a failed save is reported in the sidebar', async ({ page }) => {
	await install(page, {
		sections: [section({ requests: [savedRequest()] })],
		saveError: 'disk is full'
	});
	await page.goto('/');

	await page.getByText('List users').click();
	await page.getByPlaceholder('/user/get').fill('/boom');
	await expect(page.getByText(/disk is full/)).toBeVisible();
});

test('right-clicking the empty list offers to create either kind of thing', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await page.locator('[data-sidebar-scroller]').click({ button: 'right' });
	await expect(page.getByRole('menuitem', { name: 'New collection' })).toBeVisible();
	await expect(page.getByRole('menuitem', { name: 'New request' })).toBeVisible();
});

/**
 * Two collections describing the same API — staging and production — give every
 * loaded endpoint the same id, because a loaded id is `METHOD /path` and carries
 * no section. Selecting by that id alone lit both rows, resolved to whichever
 * collection sorted first, and left the second one unopenable: clicking it set
 * an id already held, so nothing changed.
 */
test('the same endpoint in two collections selects independently', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				id: 'staging',
				name: 'Staging',
				baseUrl: 'https://staging.acme.com',
				order: 0,
				loader: {
					enabled: true,
					url: '/openapi.json',
					method: 'GET',
					query: '.',
					next: '',
					ttlSeconds: 0
				}
			}),
			section({
				id: 'prod',
				name: 'Production',
				baseUrl: 'https://api.acme.com',
				order: 1,
				loader: {
					enabled: true,
					url: '/openapi.json',
					method: 'GET',
					query: '.',
					next: '',
					ttlSeconds: 0
				}
			})
		],
		loaded: [{ method: 'GET', path: '/users', name: 'List users', description: '', body: '' }]
	});
	await page.goto('/');

	const rows = page.getByText('List users', { exact: true });
	await expect(rows).toHaveCount(2);

	// Selecting in staging must not light up production's copy. `bg-raised` is
	// what marks the selected row, so exactly one of the two carries it.
	const highlighted = page.locator('.cursor-default.bg-raised', { hasText: 'List users' });

	await rows.nth(0).click();
	await expect(page.getByText('https://staging.acme.com', { exact: true })).toBeVisible();
	await expect(highlighted).toHaveCount(1);

	// And production is still reachable, which it was not when the id alone
	// decided: the click set a value the store already held.
	await rows.nth(1).click();
	await expect(page.getByText('https://api.acme.com', { exact: true })).toBeVisible();
	await expect(highlighted).toHaveCount(1);

	// Back again, to prove it is not one-way.
	await rows.nth(0).click();
	await expect(page.getByText('https://staging.acme.com', { exact: true })).toBeVisible();
});
