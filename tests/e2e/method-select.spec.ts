import { expect, test } from '@playwright/test';
import { install, savedRequest, section } from './mock-ipc';

/**
 * `request.spec.ts` covers the two ends of the method list (GET/HEAD vs.
 * POST) and their effect on the Body tab. This rounds out the picker itself:
 * the other methods, that a typed body survives being hidden and shown
 * again, and the dropdown's own keyboard and dismissal behaviour.
 */

const users = savedRequest();

test.describe('methods that keep the Body tab', () => {
	for (const method of ['PUT', 'PATCH', 'DELETE', 'OPTIONS']) {
		test(`${method} shows Body alongside Params`, async ({ page }) => {
			await install(page, { sections: [section({ requests: [users] })] });
			await page.goto('/');
			await page.getByText('List users').click();

			await page.getByLabel('HTTP method').click();
			await page.getByRole('option', { name: method, exact: true }).click();
			await expect(page.getByRole('tab', { name: 'Body' })).toBeVisible();
			await expect(page.getByRole('tab', { name: /^Params/ })).toBeVisible();
		});
	}
});

test('switching to GET and back preserves the body you had typed', async ({ page }) => {
	await install(page, {
		sections: [section({ requests: [savedRequest({ method: 'POST', path: '/users' })] })]
	});
	await page.goto('/');
	await page.getByText('List users').click();

	await page.getByRole('tab', { name: 'Body' }).click();
	const editor = page.locator('.cm-content').first();
	await editor.click();
	await page.keyboard.type('{"kept":true}');

	await page.getByLabel('HTTP method').click();
	await page.getByRole('option', { name: 'GET', exact: true }).click();
	await expect(page.getByRole('tab', { name: 'Body' })).toBeHidden();

	await page.getByLabel('HTTP method').click();
	await page.getByRole('option', { name: 'POST', exact: true }).click();
	await expect(page.getByRole('tab', { name: 'Body' })).toBeVisible();
	await page.getByRole('tab', { name: 'Body' }).click();
	await expect(editor).toContainText('{"kept":true}');
});

test('the trigger is colour-coded per method, and updates when it changes', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	await page.goto('/');
	await page.getByText('List users').click();

	const trigger = page.getByLabel('HTTP method');
	const classesFor = (method: string) => trigger.getAttribute('class').then((c) => c?.split(' '));

	const getClasses = await classesFor('GET');
	await trigger.click();
	await page.getByRole('option', { name: 'DELETE', exact: true }).click();
	const deleteClasses = await classesFor('DELETE');

	expect(deleteClasses).not.toEqual(getClasses);
});

test('Escape closes the dropdown without changing the method', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	await page.goto('/');
	await page.getByText('List users').click();

	await page.getByLabel('HTTP method').click();
	await expect(page.getByRole('option', { name: 'POST', exact: true })).toBeVisible();
	await page.keyboard.press('Escape');
	await expect(page.getByRole('option', { name: 'POST', exact: true })).toBeHidden();
	await expect(page.getByLabel('HTTP method')).toContainText('GET');
});

test('the arrow keys and Enter choose a method without a mouse', async ({ page }) => {
	await install(page, { sections: [section({ requests: [users] })] });
	await page.goto('/');
	await page.getByText('List users').click();

	const trigger = page.getByLabel('HTTP method');
	await trigger.click();
	await expect(page.getByRole('option', { name: 'POST', exact: true })).toBeVisible();
	// GET is highlighted first; one press should reach POST, next in the list.
	await page.keyboard.press('ArrowDown');
	await page.keyboard.press('Enter');

	await expect(trigger).toContainText('POST');
	await expect(page.getByRole('tab', { name: 'Body' })).toBeVisible();
});

test('the selected method shows a checkmark in the list', async ({ page }) => {
	await install(page, {
		sections: [section({ requests: [savedRequest({ method: 'DELETE', path: '/users/1' })] })]
	});
	await page.goto('/');
	await page.getByText('List users').click();

	await page.getByLabel('HTTP method').click();
	const selected = page.getByRole('option', { name: 'DELETE', exact: true });
	await expect(selected.locator('.i-lucide-check')).toBeVisible();
	await expect(
		page.getByRole('option', { name: 'GET', exact: true }).locator('.i-lucide-check')
	).toBeHidden();
});
