import { expect, type Locator, type Page, test } from '@playwright/test';
import { commands, install, savedRequest, section } from './mock-ipc';
import { dragAndDrop, dragOver } from './dnd-utils';

/**
 * Drag-and-drop in the sidebar, covering both kinds of thing that move:
 * requests (reordered within a collection, or moved into another) and
 * collections themselves (reordered among the others).
 *
 * These simulate the native HTML5 drag events the underlying library reads —
 * see `dnd-utils.ts` for why Playwright's own drag helpers don't apply here.
 */

/**
 * Both a request row and a collection header wear `.draggable-row` — they're
 * both drop targets — so telling them apart needs more than the class. Only
 * a header's trigger draws the collapse chevron.
 */
function requestRows(page: Page): Locator {
	return page.locator('.draggable-row').filter({ hasNot: page.locator('.i-lucide-chevron-right') });
}

function collectionHeaders(page: Page): Locator {
	return page.locator('.draggable-row').filter({ has: page.locator('.i-lucide-chevron-right') });
}

test.describe('reordering requests within a collection', () => {
	test('dropping above a row moves the dragged request before it', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' }),
						savedRequest({ id: 'r2', name: 'Beta', path: '/beta' }),
						savedRequest({ id: 'r3', name: 'Gamma', path: '/gamma' })
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('Gamma').waitFor();

		const rows = requestRows(page);
		await dragAndDrop(
			page,
			rows.filter({ hasText: 'Gamma' }),
			rows.filter({ hasText: 'Alpha' }),
			'top'
		);

		await expect(rows).toHaveText([/Gamma/, /Alpha/, /Beta/]);
	});

	test('dropping below a row moves the dragged request after it', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' }),
						savedRequest({ id: 'r2', name: 'Beta', path: '/beta' })
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('Beta').waitFor();

		const rows = requestRows(page);
		await dragAndDrop(
			page,
			rows.filter({ hasText: 'Alpha' }),
			rows.filter({ hasText: 'Beta' }),
			'bottom'
		);

		await expect(rows).toHaveText([/Beta/, /Alpha/]);
	});

	test('the new order is written to disk', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' }),
						savedRequest({ id: 'r2', name: 'Beta', path: '/beta' })
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('Beta').waitFor();

		const rows = requestRows(page);
		await dragAndDrop(
			page,
			rows.filter({ hasText: 'Alpha' }),
			rows.filter({ hasText: 'Beta' }),
			'bottom'
		);

		await expect
			.poll(async () => {
				const saved = await page.evaluate(() => window.__FIBER_TEST__.lastSaved);
				return (saved as { requests?: { name: string }[] } | null)?.requests?.map((r) => r.name);
			})
			.toEqual(['Beta', 'Alpha']);
	});

	test('dropping a row onto itself changes nothing', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' }),
						savedRequest({ id: 'r2', name: 'Beta', path: '/beta' })
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('Beta').waitFor();

		const rows = requestRows(page);
		await dragAndDrop(
			page,
			rows.filter({ hasText: 'Alpha' }),
			rows.filter({ hasText: 'Alpha' }),
			'bottom'
		);
		await page.waitForTimeout(200);

		await expect(rows).toHaveText([/Alpha/, /Beta/]);
		expect(await commands(page, 'save_section')).toEqual([]);
	});
});

test.describe('moving a request between collections', () => {
	test('dropping a request onto another collection header moves it to the end of that list', async ({
		page
	}) => {
		await install(page, {
			sections: [
				section({
					requests: [savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' })]
				}),
				section({
					id: 'sec-2',
					name: 'Other',
					order: 1,
					requests: [savedRequest({ id: 'r2', name: 'Existing', path: '/existing' })]
				})
			]
		});
		await page.goto('/');
		await page.getByText('Existing').waitFor();

		const source = requestRows(page).filter({ hasText: 'Alpha' });
		const target = collectionHeaders(page).filter({ hasText: 'Other' });
		await dragAndDrop(page, source, target, 'center');

		// The source collection is left empty; the destination gains the request
		// at the end of its own list, after what was already there.
		await expect(page.getByLabel('0 endpoints')).toBeVisible();
		await expect(page.getByLabel('2 endpoints')).toBeVisible();
		await expect(page.getByText('No requests yet.')).toBeVisible();

		const otherRows = requestRows(page).filter({ hasText: /Existing|Alpha/ });
		await expect(otherRows).toHaveText([/Existing/, /Alpha/]);
	});

	test('both the source and destination collections are saved', async ({ page }) => {
		await install(page, {
			sections: [
				section({ requests: [savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' })] }),
				section({ id: 'sec-2', name: 'Other', order: 1, requests: [] })
			]
		});
		await page.goto('/');
		await page.getByText('Other').waitFor();

		const source = requestRows(page).filter({ hasText: 'Alpha' });
		const target = collectionHeaders(page).filter({ hasText: 'Other' });
		await dragAndDrop(page, source, target, 'center');

		await expect.poll(async () => (await commands(page, 'save_section')).length).toBeGreaterThanOrEqual(2);
	});

	test('dropping a request onto its own collection header does nothing', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' }),
						savedRequest({ id: 'r2', name: 'Beta', path: '/beta' })
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('Beta').waitFor();

		const source = requestRows(page).filter({ hasText: 'Alpha' });
		const target = collectionHeaders(page).filter({ hasText: 'Acme' });
		await dragAndDrop(page, source, target, 'center');
		await page.waitForTimeout(200);

		await expect(page.getByLabel('2 endpoints')).toBeVisible();
		expect(await commands(page, 'save_section')).toEqual([]);
	});

	test('a loose request can be dragged into a collection', async ({ page }) => {
		await install(page, {
			sections: [
				{
					id: 'loose',
					name: 'Loose requests',
					baseUrl: '',
					collapsed: false,
					order: -1,
					auth: { kind: 'none' },
					mcp: { enabled: false, allowWrites: false },
					timeoutMs: 60_000,
					followRedirects: true,
					acceptInvalidCerts: false,
					proxy: '',
					requests: [savedRequest({ id: 'r1', name: 'Loose one', path: '/loose' })],
					overlay: []
				},
				section({ requests: [] })
			]
		});
		await page.goto('/');
		await page.getByText('Loose one').waitFor();

		const source = requestRows(page).filter({ hasText: 'Loose one' });
		const target = collectionHeaders(page).filter({ hasText: 'Acme' });
		await dragAndDrop(page, source, target, 'center');

		await expect(page.getByLabel('1 endpoint')).toBeVisible();
		await expect(page.getByText('Loose one')).toBeVisible();
	});
});

test.describe('reordering collections', () => {
	test('dragging a header above another reorders them', async ({ page }) => {
		await install(page, {
			sections: [
				section({ id: 'sec-1', name: 'Alpha', order: 0 }),
				section({ id: 'sec-2', name: 'Beta', order: 1 }),
				section({ id: 'sec-3', name: 'Gamma', order: 2 })
			]
		});
		await page.goto('/');
		await page.getByText('Gamma', { exact: true }).waitFor();

		const headers = collectionHeaders(page);
		await dragAndDrop(
			page,
			headers.filter({ hasText: 'Gamma' }),
			headers.filter({ hasText: 'Alpha' }),
			'top'
		);

		await expect(headers).toHaveText([/Gamma/, /Alpha/, /Beta/]);
	});

	test('dragging a header below another reorders them', async ({ page }) => {
		await install(page, {
			sections: [
				section({ id: 'sec-1', name: 'Alpha', order: 0 }),
				section({ id: 'sec-2', name: 'Beta', order: 1 })
			]
		});
		await page.goto('/');
		await page.getByText('Beta', { exact: true }).waitFor();

		const headers = collectionHeaders(page);
		await dragAndDrop(
			page,
			headers.filter({ hasText: 'Alpha' }),
			headers.filter({ hasText: 'Beta' }),
			'bottom'
		);

		await expect(headers).toHaveText([/Beta/, /Alpha/]);
	});

	test('the new order is renumbered and saved for both sections', async ({ page }) => {
		await install(page, {
			sections: [
				section({ id: 'sec-1', name: 'Alpha', order: 0 }),
				section({ id: 'sec-2', name: 'Beta', order: 1 })
			]
		});
		await page.goto('/');
		await page.getByText('Beta', { exact: true }).waitFor();

		const headers = collectionHeaders(page);
		await dragAndDrop(
			page,
			headers.filter({ hasText: 'Alpha' }),
			headers.filter({ hasText: 'Beta' }),
			'bottom'
		);

		await expect
			.poll(async () => {
				const saved = await commands(page, 'save_section');
				return saved.map((call) => [
					(call.args.section as { name?: string }).name,
					(call.args.section as { order?: number }).order
				]);
			})
			.toEqual(
				expect.arrayContaining([
					['Beta', 0],
					['Alpha', 1]
				])
			);
	});

	test('dropping a collection header onto itself changes nothing', async ({ page }) => {
		await install(page, {
			sections: [
				section({ id: 'sec-1', name: 'Alpha', order: 0 }),
				section({ id: 'sec-2', name: 'Beta', order: 1 })
			]
		});
		await page.goto('/');
		await page.getByText('Beta', { exact: true }).waitFor();

		const headers = collectionHeaders(page);
		await dragAndDrop(
			page,
			headers.filter({ hasText: 'Alpha' }),
			headers.filter({ hasText: 'Alpha' }),
			'bottom'
		);
		await page.waitForTimeout(200);

		await expect(headers).toHaveText([/Alpha/, /Beta/]);
		expect(await commands(page, 'save_section')).toEqual([]);
	});
});

test.describe('drop indicators', () => {
	test('hovering the top half of a row shows the line above it', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' }),
						savedRequest({ id: 'r2', name: 'Beta', path: '/beta' })
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('Beta').waitFor();

		const source = requestRows(page).filter({ hasText: 'Beta' });
		const target = requestRows(page).filter({ hasText: 'Alpha' });
		const release = await dragOver(page, source, target, 'top');

		await expect(target).toHaveClass(/drop-above/);
		await release();
	});

	test('hovering the bottom half of the last row shows the line below it', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' }),
						savedRequest({ id: 'r2', name: 'Beta', path: '/beta' })
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('Beta').waitFor();

		const source = requestRows(page).filter({ hasText: 'Alpha' });
		const target = requestRows(page).filter({ hasText: 'Beta' });
		const release = await dragOver(page, source, target, 'bottom');

		await expect(target).toHaveClass(/drop-below/);
		await release();
	});

	test('hovering a foreign collection header while dragging a request shows drop-into', async ({
		page
	}) => {
		await install(page, {
			sections: [
				section({ requests: [savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' })] }),
				section({ id: 'sec-2', name: 'Other', order: 1, requests: [] })
			]
		});
		await page.goto('/');
		await page.getByText('Other').waitFor();

		const source = requestRows(page).filter({ hasText: 'Alpha' });
		const target = collectionHeaders(page).filter({ hasText: 'Other' });
		const release = await dragOver(page, source, target, 'center');

		await expect(target).toHaveClass(/drop-into/);
		await release();
	});

	test('releasing outside any target leaves the list untouched', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						savedRequest({ id: 'r1', name: 'Alpha', path: '/alpha' }),
						savedRequest({ id: 'r2', name: 'Beta', path: '/beta' })
					]
				})
			]
		});
		await page.goto('/');
		await page.getByText('Beta').waitFor();

		const rows = requestRows(page);
		const source = rows.filter({ hasText: 'Alpha' });
		const target = rows.filter({ hasText: 'Beta' });
		const release = await dragOver(page, source, target, 'top');
		await expect(target).toHaveClass(/drop-above/);

		await release();
		await expect(target).not.toHaveClass(/drop-above/);
		await expect(rows).toHaveText([/Alpha/, /Beta/]);
	});
});
