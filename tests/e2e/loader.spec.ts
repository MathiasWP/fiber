import { expect, test } from '@playwright/test';
import { commands, install, section } from './mock-ipc';

const loader = {
	enabled: true,
	url: '/openapi.json',
	method: 'GET',
	query: '.paths',
	next: '',
	ttlSeconds: 0
};

async function openLoader(page: import('@playwright/test').Page) {
	await page.getByLabel('Settings for Acme').click();
	await page.getByRole('tab', { name: /^Loader/ }).click();
}

test('a collection without a loader can add one', async ({ page }) => {
	await install(page, { sections: [section()] });
	await page.goto('/');
	await openLoader(page);

	await expect(page.getByText(/keeps this section's endpoints in step/)).toBeVisible();
	await page.getByRole('button', { name: 'Add a loader' }).click();
	await expect(page.getByLabel('Enabled')).toBeChecked();
	await expect(page.getByPlaceholder('/openapi.json')).toHaveValue('/openapi.json');
	await expect.poll(() => commands(page, 'default_loader')).not.toEqual([]);
	await expect(page.getByRole('tab', { name: 'Loader · on' })).toBeVisible();
});

test('Remove drops the loader and the tab goes back to a plain name', async ({ page }) => {
	await install(page, { sections: [section({ loader })] });
	await page.goto('/');
	await openLoader(page);

	await page.getByRole('button', { name: 'Remove' }).click();
	await expect(page.getByRole('button', { name: 'Add a loader' })).toBeVisible();
	await expect(page.getByRole('tab', { name: 'Loader', exact: true })).toBeVisible();
});

test('the tab says when a loader is switched off', async ({ page }) => {
	await install(page, { sections: [section({ loader: { ...loader, enabled: false } })] });
	await page.goto('/');
	await page.getByLabel('Settings for Acme').click();
	await expect(page.getByRole('tab', { name: 'Loader · off' })).toBeVisible();
});

test('Run now reports how many endpoints came back', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		loaded: [],
		refreshed: [
			{ method: 'GET', path: '/a', name: 'a', description: '', body: '' },
			{ method: 'POST', path: '/b', name: 'b', description: '', body: '' }
		]
	});
	await page.goto('/');
	await openLoader(page);

	await page.getByRole('button', { name: 'Run now' }).click();
	await expect(page.getByText('2 endpoints from 1 page · 2 new')).toBeVisible();
	await page.getByRole('button', { name: 'Done' }).click();
	await expect(page.getByText('a', { exact: true })).toBeVisible();
	await expect(page.getByText('b', { exact: true })).toBeVisible();
});

test('Fetch a sample shows the document and the live preview', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		probeDocument: { hello: 'sample' },
		previewEndpoints: [{ method: 'GET', path: '/preview', name: 'fromFilter', description: '', body: '' }]
	});
	await page.goto('/');
	await openLoader(page);

	await page.getByRole('button', { name: 'Fetch a sample' }).click();
	await expect(page.getByText(/Filter runs as you type/)).toBeVisible();
	await expect(page.getByText('"hello": "sample"')).toBeVisible();
	await expect(page.getByText('/preview')).toBeVisible();
	await expect(page.getByText('fromFilter')).toBeVisible();
	await expect(page.getByText('Endpoints (1)')).toBeVisible();
});

test('a probe failure is named', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		probeError: 'manifest 404'
	});
	await page.goto('/');
	await openLoader(page);
	await page.getByRole('button', { name: 'Fetch a sample' }).click();
	await expect(page.getByText(/manifest 404/)).toBeVisible();
});

test('a filter that produces nothing says so, rather than looking stuck', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		probeDocument: { empty: true },
		previewEndpoints: []
	});
	await page.goto('/');
	await openLoader(page);
	await page.getByRole('button', { name: 'Fetch a sample' }).click();
	await expect(page.getByText('The filter produced nothing.')).toBeVisible();
});

test('a broken filter surfaces the jq error', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		probeDocument: {},
		previewError: 'jq: invalid'
	});
	await page.goto('/');
	await openLoader(page);
	await page.getByRole('button', { name: 'Fetch a sample' }).click();
	await expect(page.getByText(/jq: invalid/)).toBeVisible();
});

test('an endpoint the loader drops can be forgotten', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				loader,
				overlay: [
					{
						id: 'POST /gone',
						name: 'gone',
						method: 'POST',
						path: '/gone',
						body: '{"keep":false}',
						headers: []
					}
				]
			})
		],
		loaded: [{ method: 'GET', path: '/still', name: 'still', description: '', body: '' }],
		refreshed: [{ method: 'GET', path: '/still', name: 'still', description: '', body: '' }]
	});
	await page.goto('/');

	await expect(page.getByText('gone')).toBeVisible();
	await page.getByText('gone').click({ button: 'right' });
	await page.getByRole('menuitem', { name: 'Forget this endpoint' }).click();
	await expect(page.getByText('gone')).toBeHidden();
	await expect(page.getByText('still')).toBeVisible();
});
