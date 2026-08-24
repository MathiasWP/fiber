import { expect, test } from '@playwright/test';
import { install, response, savedRequest, section } from './mock-ipc';

/**
 * Opening where you left off — `src/lib/session.svelte.ts`.
 *
 * A reload stands in for a relaunch: the webview starts from nothing either
 * way, and the mock backend replies with the same fixtures both times, which
 * is exactly what a real restart onto the same files looks like.
 */

const users = savedRequest();

const loader = {
	enabled: true,
	url: '/openapi.json',
	method: 'GET',
	query: '.paths',
	next: '',
	ttlSeconds: 0
};

test('the endpoint that was open is open again', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	await page.goto('/');
	await page.getByText('List users').click();
	await expect(page.getByPlaceholder('/user/get')).toHaveValue('/users');

	await page.reload();
	// No click this time: the request pane comes back on the same request.
	await expect(page.getByPlaceholder('/user/get')).toHaveValue('/users');
	await expect(page.getByText('https://api.acme.com', { exact: true })).toBeVisible();
});

test('the scratch request comes back with what was typed into it', async ({ page }) => {
	await install(page);
	await page.goto('/');

	await page.getByPlaceholder('https://api.example.com/users').fill('https://example.com/ping');

	await page.reload();
	await expect(page.getByPlaceholder('https://api.example.com/users')).toHaveValue(
		'https://example.com/ping'
	);
	await expect(page.getByRole('button', { name: 'Send' })).toBeEnabled();
});

test('the sidebar tab that was open is open again', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	await page.goto('/');
	await page.getByRole('button', { name: 'History', exact: true }).click();
	await expect(page.getByPlaceholder('Search history…')).toBeVisible();

	await page.reload();
	await expect(page.getByPlaceholder('Search history…')).toBeVisible();
});

test('the response tab that was open is open again', async ({ page }) => {
	await install(page, {
		sendResponse: response({ body: '{"hello":"world"}' }),
		sections: [section({ requests: [users] })]
	});
	await page.goto('/');
	await page.getByText('List users').click();
	await page.getByRole('button', { name: 'Send' }).click();
	await page.getByRole('tab', { name: 'Raw' }).click();
	await expect(page.getByRole('tab', { name: 'Raw' })).toHaveAttribute('data-state', 'active');

	await page.reload();
	await page.getByText('List users').click();
	await page.getByRole('button', { name: 'Send' }).click();
	await expect(page.getByRole('tab', { name: 'Raw' })).toHaveAttribute('data-state', 'active');
});

test('a folder that was opened is still open', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		loaded: [
			{
				method: 'GET',
				path: '/users',
				name: 'List users',
				description: '',
				tag: 'Users',
				body: ''
			}
		]
	});
	await page.goto('/');

	// Folders start closed, so the endpoint inside is not on screen yet.
	await expect(page.getByText('List users')).toBeHidden();
	await page.getByRole('button', { name: /^Users/ }).click();
	await expect(page.getByText('List users')).toBeVisible();

	await page.reload();
	await expect(page.getByText('List users')).toBeVisible();
});

test('a stored selection whose request is gone falls back to scratch', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	// A request deleted in another window, or one a loader has stopped
	// reporting: the id resolves to nothing, and the pane must not be left
	// blank waiting for it.
	await page.addInitScript(() =>
		localStorage.setItem(
			'fiber:session',
			JSON.stringify({ requestId: 'long-gone', sectionId: 'sec-1' })
		)
	);
	await page.goto('/');

	await expect(page.getByPlaceholder('https://api.example.com/users')).toBeVisible();
	await expect(page.getByRole('button', { name: 'Send' })).toBeDisabled();
});

test('a session blob that is nonsense is ignored rather than fatal', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	await page.addInitScript(() => localStorage.setItem('fiber:session', '{not json'));
	await page.goto('/');

	await expect(page.getByText('List users')).toBeVisible();
	await expect(page.getByPlaceholder('https://api.example.com/users')).toBeVisible();
});

test('an update restart saves the pending edit before relaunching', async ({ page }) => {
	await install(page, {
		update: { version: '1.2.3' },
		sections: [section({ requests: [users] })]
	});
	await page.goto('/');
	await page.getByText('List users').click();

	// Inside the save debounce, which is the whole point: a restart nobody
	// asked for lands mid-edit, and the write has to be forced before the
	// process goes away.
	await page.getByPlaceholder('/user/get').fill('/edited');
	await page.getByRole('button', { name: 'Update' }).click();

	await expect.poll(() => order(page)).toEqual({ saved: true, savedBeforeRestart: true });
	const saved = await page.evaluate(() => window.__FIBER_TEST__.lastSaved);
	expect(JSON.stringify(saved)).toContain('/edited');
});

/** Where the last save landed relative to the restart, if both happened. */
function order(page: import('@playwright/test').Page) {
	return page.evaluate(() => {
		const cmds = window.__FIBER_TEST__.calls.map((call) => call.cmd);
		const restart = cmds.indexOf('plugin:process|restart');
		const saved = cmds.lastIndexOf('save_section');
		return { saved: saved !== -1, savedBeforeRestart: restart !== -1 && saved < restart };
	});
}
