import { expect, test } from '@playwright/test';
import { install, response, section, TEMPLATES } from './mock-ipc';

const loader = {
	enabled: true,
	url: '/openapi.json',
	method: 'GET',
	query: '.paths',
	next: '',
	ttlSeconds: 0
};

test.describe('the collection header', () => {
	test('the cog opens section settings', async ({ page }) => {
		await install(page, { sections: [section()] });
		await page.goto('/');

		await expect(page.getByText('Section settings')).toBeHidden();
		await page.getByLabel('Settings for Acme').click();
		await expect(page.getByText('Section settings')).toBeVisible();
	});

	test('the cog does not also collapse the collection', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						{ id: 'r1', name: 'List users', method: 'GET', path: '/users', body: '', headers: [] }
					]
				})
			]
		});
		await page.goto('/');

		await expect(page.getByText('List users')).toBeVisible();
		await page.getByLabel('Settings for Acme').click();
		// Still expanded: the click was the cog's, not the row's.
		await expect(page.getByText('List users')).toBeVisible();
	});
});

/**
 * The regression this exists for: dialogs carried no z-index at all, so the
 * credential picker opened *behind* the settings drawer that launched it.
 * Playwright refuses to click through a covering element, so the click is the
 * assertion — this times out with "intercepts pointer events" if the layering
 * regresses.
 */
test('the credential picker opens in front of the drawer that launched it', async ({ page }) => {
	await install(page, {
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

	await page.getByLabel('Settings for Acme').click();
	await page.getByRole('tab', { name: /^Auth/ }).click();
	await page.getByRole('button', { name: 'Pick credential…' }).click();

	await expect(page.getByText('Pick your credential')).toBeVisible();

	/*
	 * Clicking the dialog is not enough on its own: it is centred and wider than
	 * the drawer, so most of it is over empty pane either way and the click
	 * succeeds even when the layering is wrong. This asks the question directly —
	 * at a point both of them cover, which one does the browser hand back?
	 */
	const stacking = await page.evaluate(() => {
		const drawer = document.querySelector('.drawer');
		const picker = Array.from(document.querySelectorAll('[role="dialog"]')).find((el) =>
			el.textContent?.includes('Pick your credential')
		);
		if (!drawer || !picker) return { found: false } as const;

		const a = drawer.getBoundingClientRect();
		const b = picker.getBoundingClientRect();
		const x = Math.max(a.left, b.left) + 4;
		const y = Math.max(a.top, b.top) + 4;
		const overlap = x < Math.min(a.right, b.right) && y < Math.min(a.bottom, b.bottom);

		const hit = document.elementFromPoint(x, y);
		return {
			found: true,
			overlap,
			onTop: !!hit && picker.contains(hit)
		} as const;
	});

	expect(stacking.found).toBe(true);
	// Without this the assertion below could pass by never testing anything.
	expect(stacking.overlap).toBe(true);
	expect(stacking.onTop).toBe(true);

	// And it is genuinely interactive, not merely painted on top.
	const filter = page.getByPlaceholder('Filter by name, path or value…');
	await filter.fill('session');
	await expect(filter).toHaveValue('session');
});

test.describe('option-arrow in a URL field', () => {
	/**
	 * macOS treats a host as one word, so the native jump goes to the end. These
	 * assert the address-bar behaviour: a stop at every dot.
	 */
	async function urlField(page: import('@playwright/test').Page) {
		await install(page);
		await page.goto('/');
		await page.locator('button', { has: page.locator('.i-lucide-folder-plus') }).first().click();

		const field = page.getByPlaceholder('https://api.example.com', { exact: true });
		await expect(field).toBeVisible();
		await field.fill('https://app.staging.kvistsolutions.com');
		return field;
	}

	const caret = (field: import('@playwright/test').Locator) =>
		field.evaluate((node: HTMLInputElement) => node.selectionStart);

	test('forward stops at the next dot, not the end', async ({ page }) => {
		const field = await urlField(page);

		// Caret just after "app", before the dot.
		await field.evaluate((node: HTMLInputElement) => node.setSelectionRange(11, 11));
		await field.press('Alt+ArrowRight');

		// End of "staging" — not the end of the string.
		expect(await caret(field)).toBe(19);
	});

	test('backward steps one segment at a time', async ({ page }) => {
		const field = await urlField(page);

		await field.evaluate((node: HTMLInputElement) => node.setSelectionRange(19, 19));
		await field.press('Alt+ArrowLeft');
		expect(await caret(field)).toBe(12);

		await field.press('Alt+ArrowLeft');
		expect(await caret(field)).toBe(8);
	});
});

test('the response pane fills in while the body is still arriving', async ({ page }) => {
	await install(page, {
		deferSend: true,
		sections: [
			section({
				requests: [
					{ id: 'r1', name: 'List users', method: 'GET', path: '/users', body: '', headers: [] }
				]
			})
		]
	});
	await page.goto('/');

	await page.getByText('List users').click();
	await page.getByRole('button', { name: 'Send' }).click();

	// Nothing has arrived, so the pane is still waiting.
	await expect(page.getByText('Streaming')).toBeHidden();

	await page.evaluate(() => {
		window.__FIBER_TEST__.start();
		window.__FIBER_TEST__.chunk('{"first":');
	});

	await expect(page.getByText('Streaming')).toBeVisible();
	await expect(page.getByText('{"first":')).toBeVisible();

	await page.evaluate(() => window.__FIBER_TEST__.chunk('"chunk"}'));
	await expect(page.getByText('{"first":"chunk"}')).toBeVisible();

	// Settling replaces the preview with the real response.
	await page.evaluate(() =>
		window.__FIBER_TEST__.settle({
			status: 200,
			statusText: 'OK',
			finalUrl: 'https://api.acme.com/users',
			headers: [{ name: 'content-type', value: 'application/json' }],
			isBinary: false,
			truncated: false,
			sizeBytes: 17,
			timing: { ttfbMs: 5, totalMs: 9 },
			body: '{"first":"chunk"}'
		})
	);

	await expect(page.getByText('Streaming')).toBeHidden();
	await expect(page.getByRole('tab', { name: 'Pretty' })).toBeVisible();
	await expect(page.getByText('200')).toBeVisible();
});

test.describe('the templates dropdown', () => {
	const [openapiName, openapiQuery] = TEMPLATES[0];

	test('names the template in use and ticks it in the list', async ({ page }) => {
		await install(page, {
			sections: [section({ loader: { ...loader, query: openapiQuery } })]
		});
		await page.goto('/');

		await page.getByLabel('Settings for Acme').click();
		await page.getByRole('tab', { name: /^Loader/ }).click();

		// The trigger says which one, rather than just "Templates".
		const trigger = page.getByRole('button', { name: openapiName });
		await expect(trigger).toBeVisible();

		await trigger.click();
		const options = page.getByRole('option');
		await expect(options.first()).toContainText(openapiName);

		// And the one in use is the one marked.
		await expect(options.first()).toHaveAttribute('aria-selected', 'true');
		// Unselected items carry no aria-selected at all, rather than 'false'.
		await expect(options.nth(1)).not.toHaveAttribute('aria-selected', 'true');
	});

	test('reads as Custom once the filter no longer matches one', async ({ page }) => {
		await install(page, {
			sections: [section({ loader: { ...loader, query: '.paths | keys' } })]
		});
		await page.goto('/');

		await page.getByLabel('Settings for Acme').click();
		await page.getByRole('tab', { name: /^Loader/ }).click();

		await expect(page.getByRole('button', { name: 'Custom' })).toBeVisible();
	});
});

/**
 * The guarantee the overlay model exists for, in the README's words: "Losing a
 * body to a refresh is the one thing this design exists to prevent."
 *
 * Loaded endpoints are a cache, not the truth. What you type against one is
 * yours, matched back by `METHOD /path`, and an endpoint the API stops
 * reporting is marked rather than deleted.
 */
test('a body written against a loaded endpoint survives the endpoint vanishing', async ({
	page
}) => {
	const created = {
		method: 'POST',
		path: '/users',
		name: 'createUser',
		description: '',
		body: ''
	};

	await install(page, {
		sections: [section({ loader })],
		loaded: [created],
		// The refresh no longer reports it.
		refreshed: []
	});
	await page.goto('/');

	// Selecting is what promotes a loaded endpoint into the overlay, so this is
	// the step that gives what follows somewhere to live.
	await page.getByText('createUser').click();

	const editor = page.locator('.cm-content').first();
	await editor.click();
	await page.keyboard.type('keep-me');
	await expect(editor).toContainText('keep-me');

	await page.getByText('Acme', { exact: true }).click({ button: 'right' });
	await page.getByRole('menuitem', { name: 'Refresh endpoints' }).click();

	// Still listed, and flagged rather than quietly dropped.
	await expect(page.getByTitle('No longer reported by the loader').first()).toBeVisible();
	await expect(page.getByText('createUser')).toBeVisible();

	// And the work is still there.
	await expect(page.locator('.cm-content').first()).toContainText('keep-me');
});

test('Done runs the loader, so a filter just edited takes effect', async ({ page }) => {
	await install(page, { sections: [section({ loader })], loaded: [] });
	await page.goto('/');

	await page.getByLabel('Settings for Acme').click();

	const ran = () => page.evaluate(() => window.__FIBER_TEST__.calls.some((c) => c.cmd === 'run_loader'));
	expect(await ran(), 'nothing should have run just from opening settings').toBe(false);

	await page.getByRole('button', { name: 'Done' }).click();
	await expect.poll(ran, { message: 'Done should refresh the endpoints' }).toBe(true);
});
