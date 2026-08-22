import { expect, test } from '@playwright/test';
import { fuzzyScore, hasRun, order, SCATTERED } from '../../src/lib/search';

test('a run always scores below a scattered match', () => {
	const run = fuzzyScore('/users/list', '/list');
	const scattered = fuzzyScore('/plugins/installation/invoke', '/list');
	expect(run).not.toBeNull();
	expect(scattered).not.toBeNull();
	expect(run!).toBeLessThan(SCATTERED);
	expect(scattered!).toBeGreaterThanOrEqual(SCATTERED);
	expect(run!).toBeLessThan(scattered!);
});

test('the earlier run wins, then the shorter haystack', () => {
	expect(fuzzyScore('list', 'list')!).toBeLessThan(fuzzyScore('xxlist', 'list')!);
	expect(fuzzyScore('list', 'list')!).toBeLessThan(fuzzyScore('lists', 'list')!);
});

test('no match at all is null, not a bad score', () => {
	expect(fuzzyScore('abc', 'z')).toBeNull();
	expect(fuzzyScore('abc', '')).toBe(0);
});

test('order drops scattered matches when anything matched properly', () => {
	const scored = [
		{ item: 'noise', score: SCATTERED + 10 },
		{ item: 'hit', score: 12 }
	];
	expect(order(scored, hasRun(scored))).toEqual(['hit']);
	expect(order(scored, false).map((item) => item)).toEqual(['hit', 'noise']);
});
