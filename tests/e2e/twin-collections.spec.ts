import { expect, test } from '@playwright/test';
import { commands, install, response, section } from './mock-ipc';

/**
 * Two collections describing one API — staging and production — which is how
 * anybody works on an API they also run.
 *
 * Every loaded endpoint has the same id in both, because a loaded id is
 * `METHOD /path` and deliberately carries no section: it is the identity a
 * saved body and a refresh must agree on. Anything else keyed on that id alone
 * therefore cannot tell the two collections apart, and this file is where that
 * assumption gets tested rather than assumed.
 */

const loader = {
	enabled: true,
	url: '/openapi.json',
	method: 'GET',
	query: '.',
	next: '',
	ttlSeconds: 0
};

const twins = [
	section({
		id: 'staging',
		name: 'Staging',
		baseUrl: 'https://staging.acme.com',
		order: 0,
		loader
	}),
	section({
		id: 'prod',
		name: 'Production',
		baseUrl: 'https://api.acme.com',
		order: 1,
		loader
	})
];

const oneEndpoint = [
	{ method: 'GET', path: '/users', name: 'List users', description: '', body: '' }
];

/** The staging row, then the production row. */
function rows(page: import('@playwright/test').Page) {
	return page.getByText('List users', { exact: true });
}

test('a response in one collection does not appear under the other', async ({ page }) => {
	await install(page, {
		sections: twins,
		loaded: oneEndpoint,
		sendResponse: response({ status: 201, statusText: 'Created' })
	});
	await page.goto('/');

	await rows(page).nth(0).click();
	await page.getByRole('button', { name: 'Send' }).click();
	await expect(page.getByText('201 Created')).toBeVisible();

	// Production has never been sent. Its pane should be offering to send, not
	// showing staging's reply.
	await rows(page).nth(1).click();
	await expect(page.getByText('Send a request to see the response.')).toBeVisible();
	await expect(page.getByText('201 Created')).toBeHidden();
});

test('clearing one collection history leaves the other alone', async ({ page }) => {
	await install(page, {
		sections: twins,
		loaded: oneEndpoint,
		sendResponse: response()
	});
	await page.goto('/');

	await rows(page).nth(0).click();
	await page.getByRole('button', { name: 'Send' }).click();
	await expect(page.getByText('200 OK')).toBeVisible();

	await rows(page).nth(1).click();
	await page.getByRole('button', { name: 'Send' }).click();
	await expect(page.getByText('200 OK')).toBeVisible();

	// Whatever the UI does, the backend must not be asked to delete a bucket
	// that holds both collections' entries under one key.
	const cleared = await commands(page, 'history_clear_request');
	expect(cleared).toHaveLength(0);

	// The request id is deliberately the same in both — it is the endpoint's
	// identity, and a refresh has to re-attach to it. What tells the two apart,
	// and what the history bucket is keyed on, is the section travelling beside
	// it.
	const sent = await commands(page, 'send_request');
	expect(sent).toHaveLength(2);
	const specs = sent.map((call) => call.args.spec as { requestId: string; sectionId: string });
	expect(specs.map((spec) => spec.requestId)).toEqual(['GET /users', 'GET /users']);
	expect(specs.map((spec) => spec.sectionId)).toEqual(['staging', 'prod']);
});

test('editing a body in one collection does not change the other', async ({ page }) => {
	await install(page, {
		sections: twins,
		loaded: [
			{
				method: 'POST',
				path: '/users',
				name: 'Create user',
				description: '',
				body: '{\n  "name": "string"\n}'
			}
		]
	});
	await page.goto('/');

	const created = page.getByText('Create user', { exact: true });
	await created.nth(0).click();
	await page.getByRole('tab', { name: 'Body' }).click();

	const editor = page.locator('.cm-content').first();
	await editor.click();
	await page.keyboard.press('ControlOrMeta+a');
	await page.keyboard.type('{"name": "staging only"}');

	await created.nth(1).click();
	await page.getByRole('tab', { name: 'Body' }).click();
	await expect(page.locator('.cm-content').first()).not.toContainText('staging only');
});

test('forgetting an endpoint in one collection leaves the other showing it', async ({ page }) => {
	const orphan = {
		id: 'POST /gone',
		name: 'gone',
		method: 'POST',
		path: '/gone',
		body: '',
		headers: []
	};
	await install(page, {
		sections: [
			section({ ...twins[0], overlay: [orphan] }),
			section({ ...twins[1], overlay: [orphan] })
		],
		loaded: oneEndpoint
	});
	await page.goto('/');

	// An endpoint the loader no longer reports, orphaned in both collections
	// under the same id — so forgetting it in one must not take the other's.
	const gone = page.getByText('gone', { exact: true });
	await expect(gone).toHaveCount(2);

	await gone.nth(0).click({ button: 'right' });
	await page.getByRole('menuitem', { name: 'Forget this endpoint' }).click();
	await expect(gone).toHaveCount(1);
});

test('opening an endpoint asks for its own collection schema', async ({ page }) => {
	await install(page, { sections: twins, loaded: oneEndpoint });
	await page.goto('/');

	await rows(page).nth(1).click();
	await expect(page.getByText('https://api.acme.com', { exact: true })).toBeVisible();

	// The schema is per collection. Asking staging's for a row clicked in
	// production would show the wrong document's fields against it.
	const asked = await commands(page, 'loader_schema');
	expect(asked.length).toBeGreaterThan(0);
	const last = asked[asked.length - 1].args as { sectionId: string; endpointId: string };
	expect(last.sectionId).toBe('prod');
	expect(last.endpointId).toBe('GET /users');
});
