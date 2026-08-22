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

/**
 * One corrupt TOML file must not take the rest of the collections down — but
 * skipping it silently reads as data loss. The file is named, the parse error
 * quoted, and the good sections load around it.
 */
test('a collection file that cannot be read is reported, not swallowed', async ({ page }) => {
	await install(page, {
		sections: [section()],
		sectionErrors: [{ file: 'books.toml', message: 'expected `=` at line 3' }]
	});
	await page.goto('/');

	// The good section still loads…
	await expect(page.getByText('Acme', { exact: true })).toBeVisible();
	// …and the bad file is named, with the reassurance that it is still on disk.
	const notice = page.getByText(/Skipped 1 collection file/);
	await expect(notice).toBeVisible();
	await expect(notice).toContainText('books.toml');
	await expect(notice).toContainText('untouched on disk');
});

test.describe('the collection header', () => {
	test('loads more endpoints while you scroll', async ({ page }) => {
		const requests = Array.from({ length: 102 }, (_, index) => ({
			id: `r-${index}`,
			name: `Request ${index + 1}`,
			method: 'GET',
			path: `/requests/${index + 1}`,
			body: '',
			headers: []
		}));
		await install(page, { sections: [section({ requests })] });
		await page.goto('/');

		await expect(page.getByText('Request 100', { exact: true })).toBeVisible();
		await expect(page.getByText('Request 101', { exact: true })).toBeHidden();
		await page.getByText('Request 100', { exact: true }).scrollIntoViewIfNeeded();
		await expect(page.getByText('Request 102', { exact: true })).toBeVisible();
	});

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

/**
 * The reported gap: endpoints discovered by a loader arrived with an empty
 * editor, because a jq filter maps shape to shape and knows nothing about JSON
 * Schema. The body is derived in Rust now; this covers the wiring that carries
 * it to the editor.
 */
test('a loaded endpoint arrives with the body its manifest declared', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		loaded: [
			{
				method: 'POST',
				path: '/activity/backfill-activity',
				name: 'activity_backfill_activity',
				description: '',
				body: '{\n  "offset": 0,\n  "dryRun": false\n}'
			}
		]
	});
	await page.goto('/');

	await page.getByText('activity_backfill_activity').click();
	await expect(page.locator('.cm-content').first()).toContainText('"dryRun"');
});

test('shows an OpenAPI error when a request body has the wrong field type', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		loaded: [
			{
				method: 'POST',
				path: '/flags',
				name: 'setFlags',
				description: '',
				body: '{ "enabled": true }'
			}
		],
		schemas: {
			'POST /flags': {
				type: 'object',
				required: ['enabled'],
				properties: { enabled: { type: 'boolean' } }
			}
		}
	});
	await page.goto('/');
	await page.getByText('setFlags').click();

	const editor = page.locator('.cm-content').first();
	await editor.click();
	await page.keyboard.press('ControlOrMeta+a');
	await page.keyboard.type('{ "enabled": null }');

	const error = page.getByRole('alert');
	await expect(error).toContainText('Request body does not match the OpenAPI schema');
	await expect(error).toContainText('$.enabled must be boolean, not null.');
});

test('shows an OpenAPI error when a body matches two oneOf variants', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		loaded: [
			{
				method: 'POST',
				path: '/value',
				name: 'setValue',
				description: '',
				body: '{ "n": 1.5 }'
			}
		],
		schemas: {
			'POST /value': {
				type: 'object',
				properties: { n: { oneOf: [{ type: 'number' }, { type: 'integer' }] } }
			}
		}
	});
	await page.goto('/');
	await page.getByText('setValue').click();

	const editor = page.locator('.cm-content').first();
	await editor.click();
	await page.keyboard.press('ControlOrMeta+a');
	await page.keyboard.type('{ "n": 1 }');

	const error = page.getByRole('alert');
	await expect(error).toContainText('Request body does not match the OpenAPI schema');
	await expect(error).toContainText('$.n must match exactly one allowed schema.');
});

/**
 * Filling a generated body in is destructive to the placeholders that guided
 * it. Reset is the way back: the manifest still holds the skeleton, so one
 * click restores it — and with it the tabbable gaps.
 */
test('Reset restores the generated body after it has been edited', async ({ page }) => {
	const skeleton = '{\n  "offset": number,\n  "dryRun": boolean\n}';
	await install(page, {
		sections: [section({ loader })],
		loaded: [
			{
				method: 'POST',
				path: '/activity/backfill-activity',
				name: 'activity_backfill_activity',
				description: '',
				body: skeleton
			}
		]
	});
	await page.goto('/');

	await page.getByText('activity_backfill_activity').click();
	const editor = page.locator('.cm-content').first();
	await expect(editor).toContainText('"dryRun"');

	// Pristine body: nothing to reset yet.
	const reset = page.getByRole('button', { name: 'Reset' });
	await expect(reset).toBeDisabled();

	// Type the offset in — the placeholder is gone, and Reset wakes up.
	await editor.click();
	await page.keyboard.press('ControlOrMeta+a');
	await page.keyboard.type('{ "offset": 42 }');
	await expect(reset).toBeEnabled();

	await reset.click();
	await expect(editor).toContainText('"dryRun"');
	await expect(reset).toBeDisabled();
});

test.describe('searching the collections', () => {
	const many = (count: number) =>
		Array.from({ length: count }, (_, i) => ({
			id: `r${i}`,
			name: i === 0 ? 'backfill-activity' : `other-${i}`,
			method: 'POST',
			path: i === 0 ? '/activity/backfill-activity' : `/other/${i}`,
			body: '',
			headers: []
		}));

	test('says how much is hidden, and clears the filter', async ({ page }) => {
		await install(page, { sections: [section({ requests: many(20) })] });
		await page.goto('/');

		await expect(page.getByText('more hidden by filters')).toBeHidden();

		await page.getByPlaceholder('Search endpoints…').fill('backfill');

		// One of the twenty matches, so nineteen are out of sight.
		await expect(page.getByText('19 more hidden by filters')).toBeVisible();

		await page.getByRole('button', { name: 'Clear filters' }).click();
		await expect(page.getByPlaceholder('Search endpoints…')).toHaveValue('');
		await expect(page.getByText('more hidden by filters')).toBeHidden();
	});

	/** A collection with no match disappears whole — its endpoints still count. */
	test('counts collections the search removed entirely', async ({ page }) => {
		await install(page, {
			sections: [
				section({ requests: many(3) }),
				section({ id: 'sec-2', name: 'Other', order: 1, requests: many(5).slice(1) })
			]
		});
		await page.goto('/');

		await page.getByPlaceholder('Search endpoints…').fill('backfill');
		await expect(page.getByText('Other')).toBeHidden();
		// 3 + 4 endpoints, one match.
		await expect(page.getByText('6 more hidden by filters')).toBeVisible();
	});
});

test.describe('keeping endpoints current', () => {
	const withTtl = (ttlSeconds: number) => ({ ...loader, ttlSeconds });
	const runs = (page: import('@playwright/test').Page) =>
		page.evaluate(() => window.__FIBER_TEST__.calls.filter((c) => c.cmd === 'run_loader').length);

	test('coming back to the window refreshes a stale loader', async ({ page }) => {
		await install(page, { sections: [section({ loader: withTtl(60) })] });
		await page.goto('/');

		// The cached endpoints are ancient, so startup refreshes once.
		await expect.poll(() => runs(page)).toBe(1);

		await page.evaluate(() => window.dispatchEvent(new Event('focus')));
		await expect.poll(() => runs(page), { message: 'focus should refresh again' }).toBe(2);
	});

	/**
	 * The TTL field promises that 0 means "only when asked", and some APIs need
	 * it to. Focus must not quietly override that.
	 */
	test('a loader set to "only when asked" is left alone', async ({ page }) => {
		await install(page, { sections: [section({ loader: withTtl(0) })] });
		await page.goto('/');

		await page.evaluate(() => window.dispatchEvent(new Event('focus')));
		await page.waitForTimeout(150);
		expect(await runs(page)).toBe(0);
	});

	test('a collection shows it is refreshing while it happens', async ({ page }) => {
		await install(page, {
			sections: [section({ loader: withTtl(60) })],
			deferRefresh: true
		});
		await page.goto('/');

		const spinner = page.getByLabel('Refreshing endpoints');
		await expect(spinner).toBeVisible();

		await page.evaluate(() => window.__FIBER_TEST__.finishRefresh());
		await expect(spinner).toBeHidden();
	});
});

/**
 * Opening a loaded endpoint promotes it into the overlay, which is a change to
 * the collection and so a write. It used to be an immediate one, per click —
 * stringify the section, snapshot it, serialise TOML, hit the disk. The entry
 * being written holds nothing of yours yet, so those coalesce like every other
 * edit does.
 */
test('opening several endpoints writes the collection once, not once each', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		loaded: Array.from({ length: 5 }, (_, i) => ({
			method: 'POST',
			path: `/thing/${i}`,
			name: `thing-${i}`,
			description: '',
			body: ''
		}))
	});
	await page.goto('/');

	const saves = () =>
		page.evaluate(
			() => window.__FIBER_TEST__.calls.filter((c) => c.cmd === 'save_section').length
		);

	for (let i = 0; i < 5; i++) await page.getByText(`thing-${i}`).click();

	// Every one of them is a new overlay entry, and none has been written yet.
	expect(await saves()).toBe(0);

	// They land together once the typing stops.
	await expect.poll(saves, { timeout: 5000 }).toBe(1);
});

test.describe('filling in a generated body', () => {
	/** What the schema walk produces: type names where values go. */
	const generated = '{\n  "label": string,\n  "offset": number,\n  "dryRun": boolean\n}';

	const open = async (page: import('@playwright/test').Page) => {
		await install(page, {
			sections: [section({ loader })],
			loaded: [
				{ method: 'POST', path: '/x', name: 'createThing', description: '', body: generated }
			]
		});
		await page.goto('/');
		await page.getByText('createThing').click();
		const editor = page.locator('.cm-content').first();
		await expect(editor).toContainText('"label": string');
		return editor;
	};

	test('marks every unfilled field', async ({ page }) => {
		await open(page);
		// One per type name, and none for the quoted keys around them.
		await expect(page.locator('.cm-slot')).toHaveCount(3);
		await expect(page.locator('.cm-slot').first()).toHaveText('string');
	});

	test('tab moves between them and selects, so typing replaces', async ({ page }) => {
		const editor = await open(page);
		await editor.click();
		await page.keyboard.press('Tab');

		const selected = () => page.evaluate(() => window.getSelection()?.toString() ?? '');
		expect(await selected()).toBe('string');

		await page.keyboard.press('Tab');
		expect(await selected()).toBe('number');

		await page.keyboard.press('Shift+Tab');
		expect(await selected()).toBe('string');

		// Typing lands on the field rather than beside it. The quote wraps the
		// selection rather than replacing it — CodeMirror's doing, and useful
		// here: the placeholder stays selected inside the new quotes.
		await page.keyboard.type('"');
		await page.keyboard.type('hello');
		await expect(editor).toContainText('"label": "hello"');
		await expect(page.locator('.cm-slot')).toHaveCount(2);
	});

	/**
	 * This used to type the comma *inside* the string — `"hello,"` — and move on
	 * anyway, so the value it left behind was wrong every time. The closing
	 * quote is what ends the value; the comma after it is the separator, and
	 * that is the one that carries on to the next gap.
	 */
	test('a comma moves on to the next one', async ({ page }) => {
		const editor = await open(page);
		await editor.click();
		await page.keyboard.press('Tab');
		await page.keyboard.type('"hello"');

		await page.keyboard.type(',');
		await expect
			.poll(() => page.evaluate(() => window.getSelection()?.toString() ?? ''))
			.toBe('number');
		await expect(editor).toContainText('"label": "hello"');
	});

	test('leaves an ordinary body alone', async ({ page }) => {
		await install(page, {
			sections: [section({ loader })],
			loaded: [
				{
					method: 'POST',
					path: '/y',
					name: 'plainThing',
					description: '',
					body: '{\n  "label": "already filled"\n}'
				}
			]
		});
		await page.goto('/');
		await page.getByText('plainThing').click();
		await expect(page.locator('.cm-content').first()).toContainText('already filled');
		await expect(page.locator('.cm-slot')).toHaveCount(0);
	});
});

test.describe('what a search matches', () => {
	const paths = [
		'/activity/backfill-activity',
		'/plugins/installation/invoke',
		'/portfolio/asset/move',
		'/users/list'
	];

	const withPaths = async (page: import('@playwright/test').Page) => {
		await install(page, {
			sections: [
				section({
					requests: paths.map((path, i) => ({
						id: `r${i}`,
						// The OpenAPI template names endpoints by path, so that is what
						// the sidebar shows and what a search is matched against.
						name: path,
						method: 'POST',
						path,
						body: '',
						headers: []
					}))
				})
			]
		});
		await page.goto('/');
		return page.getByPlaceholder('Search endpoints…');
	};

	/**
	 * The reported case. Every one of those paths contains a `/`, an `l`, an
	 * `i`, an `s` and a `t` somewhere, so a subsequence match returns all of
	 * them — and scoring on gaps alone ranked the real hit no better.
	 */
	test('a term that matches properly hides the near-misses', async ({ page }) => {
		const search = await withPaths(page);
		await search.fill('/list');

		await expect(page.getByText('/users/list')).toBeVisible();
		await expect(page.getByText('/plugins/installation/invoke')).toBeHidden();
		await expect(page.getByText('/portfolio/asset/move')).toBeHidden();
	});

	/** But a typo has no proper match to lose to, so guessing is still useful. */
	test('a typo still finds what you meant', async ({ page }) => {
		const search = await withPaths(page);
		await search.fill('bacfkill-activ');

		await expect(page.getByText('/activity/backfill-activity')).toBeVisible();
	});
});

test.describe('collapsing while searching', () => {
	const requests = [
		{ id: 'r1', name: '/users/list', method: 'POST', path: '/users/list', body: '', headers: [] },
		{ id: 'r2', name: '/users/create', method: 'POST', path: '/users/create', body: '', headers: [] }
	];

	const search = (page: import('@playwright/test').Page) =>
		page.getByPlaceholder('Search endpoints…');

	test('a search opens a closed collection', async ({ page }) => {
		await install(page, { sections: [section({ collapsed: true, requests })] });
		await page.goto('/');

		await expect(page.getByText('/users/list')).toBeHidden();
		await search(page).fill('users');
		await expect(page.getByText('/users/list')).toBeVisible();
	});

	test('but closing it then keeps it closed', async ({ page }) => {
		await install(page, { sections: [section({ requests })] });
		await page.goto('/');

		await search(page).fill('users');
		await expect(page.getByText('/users/list')).toBeVisible();

		await page.getByText('Acme', { exact: true }).click();
		await expect(page.getByText('/users/list')).toBeHidden();

		// Refining the same search is not a new one, so it stays shut.
		await search(page).fill('users/');
		await expect(page.getByText('/users/list')).toBeHidden();
	});

	test('and a fresh search starts open again', async ({ page }) => {
		await install(page, { sections: [section({ requests })] });
		await page.goto('/');

		await search(page).fill('users');
		await page.getByText('Acme', { exact: true }).click();
		await expect(page.getByText('/users/list')).toBeHidden();

		// Clearing is what ends the search; the next one forgets what you shut.
		await search(page).fill('');
		await search(page).fill('users');
		await expect(page.getByText('/users/list')).toBeVisible();
	});

	/** Closing one to get it out of the way is about the search, not the collection. */
	test('closing during a search is not saved to the collection', async ({ page }) => {
		await install(page, { sections: [section({ requests })] });
		await page.goto('/');

		await search(page).fill('users');
		await page.getByText('Acme', { exact: true }).click();

		const saved = await page.evaluate(() =>
			window.__FIBER_TEST__.calls.filter((c) => c.cmd === 'save_section').length
		);
		expect(saved).toBe(0);

		// And the collection is still open once the search is gone.
		await search(page).fill('');
		await expect(page.getByText('/users/list')).toBeVisible();
	});
});

/**
 * An endpoint you had already opened before bodies existed has an overlay entry
 * holding an empty one, and an empty string is not nullish — so it won the
 * `??` and the schema's body never appeared. Which hit exactly the endpoints
 * you use most.
 */
test('a body from the manifest shows even if the endpoint was opened before', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				loader,
				// Opened at some point in the past, when there was no body to save.
				overlay: [
					{
						id: 'POST /things',
						name: '/things',
						method: 'POST',
						path: '/things',
						body: '',
						headers: []
					}
				]
			})
		],
		loaded: [
			{
				method: 'POST',
				path: '/things',
				name: '/things',
				description: '',
				body: '{\n  "label": string\n}'
			}
		]
	});
	await page.goto('/');

	await page.getByText('/things').click();
	await expect(page.locator('.cm-content').first()).toContainText('"label": string');
});

/** Adopting a body must never overwrite one you wrote. */
test('a body you have written survives a manifest that offers another', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				loader,
				overlay: [
					{
						id: 'POST /things',
						name: '/things',
						method: 'POST',
						path: '/things',
						body: '{"mine": true}',
						headers: []
					}
				]
			})
		],
		loaded: [
			{
				method: 'POST',
				path: '/things',
				name: '/things',
				description: '',
				body: '{\n  "label": string\n}'
			}
		]
	});
	await page.goto('/');

	await page.getByText('/things').click();
	const editor = page.locator('.cm-content').first();
	await expect(editor).toContainText('"mine"');
	await expect(editor).not.toContainText('"label": string');
});

test.describe('what the collection header tells you', () => {
	test('the count says what it counts', async ({ page }) => {
		await install(page, {
			sections: [
				section({
					requests: [
						{ id: 'r1', name: 'one', method: 'GET', path: '/one', body: '', headers: [] }
					]
				})
			]
		});
		await page.goto('/');

		await page.getByLabel('1 endpoint').hover();
		await expect(page.locator('.tooltip')).toContainText('1 endpoint');
	});

	test('the shield says whether a credential is stored', async ({ page }) => {
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

		// `has_secret` answers false in the mock, so this is the unset case.
		await page.getByLabel(/no credential stored yet/).hover();
		await expect(page.locator('.tooltip')).toContainText('no credential stored yet');
	});

	test('the cog says where it goes', async ({ page }) => {
		await install(page, { sections: [section()] });
		await page.goto('/');

		await page.getByLabel('Settings for Acme').hover();
		await expect(page.locator('.tooltip')).toContainText('Section settings');
	});
});

test('query params are available on POST', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				requests: [
					{ id: 'r1', name: 'Create user', method: 'POST', path: '/users', body: '{}', headers: [] }
				]
			})
		]
	});
	await page.goto('/');
	await page.getByText('Create user').click();

	await expect(page.getByText('Body', { exact: true })).toBeVisible();
	await page.getByText('Params', { exact: true }).click();
	const name = page.getByPlaceholder('Parameter').first();
	const value = name.locator('xpath=following-sibling::input');
	await name.fill('pretty');
	await value.fill('1');

	await expect(page.locator('input[placeholder="/user/get"]')).toHaveValue('/users?pretty=1');
});

test('OpenAPI tags become folders and descriptions appear under the URL', async ({ page }) => {
	await install(page, {
		sections: [section({ loader })],
		loaded: [
			{
				method: 'GET',
				path: '/pet',
				name: 'listPets',
				description: 'All of the pets currently in the store',
				tag: 'pet',
				body: ''
			},
			{
				method: 'GET',
				path: '/store/inventory',
				name: 'getInventory',
				description: '',
				tag: 'store',
				body: ''
			}
		]
	});
	await page.goto('/');

	await expect(page.getByText('pet', { exact: true })).toBeVisible();
	await expect(page.getByText('store', { exact: true })).toBeVisible();
	await expect(page.getByText('listPets')).toBeVisible();

	await page.getByText('listPets').click();
	await expect(page.getByText('All of the pets currently in the store')).toBeVisible();

	await page.getByRole('button', { name: /^pet/ }).click();
	await expect(page.getByText('listPets')).toBeHidden();
	await expect(page.getByText('getInventory')).toBeVisible();
});

test('a collection has HTTP settings of its own', async ({ page }) => {
	await install(page, { sections: [section()] });
	await page.goto('/');
	await page.getByLabel('Settings for Acme').click();

	await expect(page.getByText('Timeout (ms)')).toBeVisible();
	await expect(page.getByText('Follow redirects')).toBeVisible();
	await expect(page.getByText('Allow invalid TLS certificates')).toBeVisible();
	await expect(page.getByPlaceholder('http://127.0.0.1:8080')).toBeVisible();
});

test('form bodies can be chosen instead of JSON', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				requests: [
					{ id: 'r1', name: 'Create user', method: 'POST', path: '/users', body: '{}', headers: [] }
				]
			})
		]
	});
	await page.goto('/');
	await page.getByText('Create user').click();

	await page.locator('select').selectOption('form');
	await expect(page.getByPlaceholder('Field')).toBeVisible();
	await expect(page.getByText('application/x-www-form-urlencoded')).toBeVisible();
});

test('an image response is shown as a picture', async ({ page }) => {
	await install(page, {
		sections: [
			section({
				requests: [
					{ id: 'r1', name: 'Avatar', method: 'GET', path: '/avatar.png', body: '', headers: [] }
				]
			})
		],
		deferSend: true
	});
	await page.goto('/');
	await page.getByText('Avatar').click();
	await page.getByRole('button', { name: 'Send' }).click();
	await page.evaluate(() =>
		window.__FIBER_TEST__.settle({
			status: 200,
			statusText: 'OK',
			finalUrl: 'https://api.acme.com/avatar.png',
			headers: [{ name: 'content-type', value: 'image/png' }],
			isBinary: true,
			truncated: false,
			sizeBytes: 70,
			timing: { ttfbMs: 5, totalMs: 9 },
			body: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=='
		})
	);

	await expect(page.getByRole('img', { name: 'Response' })).toBeVisible();
});
