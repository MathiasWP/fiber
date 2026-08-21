/**
 * Matching a search against a list of things.
 *
 * Kept apart from the collection state deliberately: none of it touches a
 * store, which makes it the one part of search that can be reasoned about —
 * and exercised — on its own.
 */

/**
 * Anything at or above this scored as a *scattered* match: the needle's
 * characters appear in order, but not together.
 */
export const SCATTERED = 1_000_000;

export interface Scored<T> {
	item: T;
	score: number;
}

/**
 * How well `needle` matches `haystack`, lower being better, or null for no
 * match at all.
 *
 * Two tiers that never overlap. A **run** — the needle appearing whole — always
 * scores below `SCATTERED`; a scattered match always scores above it. That
 * separation is the point: scoring both on gaps alone put a real hit on
 * `/users/list` at 2025 and pure noise like `/plugins/installation/invoke` at
 * 2040, because a run late in a long path accumulates just as much gap as
 * characters strewn across one. No amount of tuning the gap weights fixes
 * that; the two kinds have to be ranked separately.
 */
export function fuzzyScore(haystack: string, needle: string): number | null {
	if (!needle) return 0;

	const target = haystack.toLowerCase();
	const query = needle.toLowerCase();

	// Earliest run wins, then the shorter of two equally placed matches.
	const run = target.indexOf(query);
	if (run !== -1) return run * 10 + target.length;

	let score = 0;
	let from = 0;
	for (const char of query) {
		const at = target.indexOf(char, from);
		if (at === -1) return null;
		// Gaps between matched characters make a result less relevant.
		score += at - from;
		from = at + 1;
	}
	return SCATTERED + score * 100 + target.length;
}

/** Whether anything here matched properly rather than by scattered letters. */
export function hasRun(scored: { score: number }[]): boolean {
	return scored.some((match) => match.score < SCATTERED);
}

/**
 * Orders matches, dropping the scattered ones when `runsOnly`.
 *
 * The caller decides that, because it is a question about the whole search
 * rather than one collection: a collection holding no real match should show
 * nothing, not fill up with near-misses because it was considered alone.
 */
export function order<T>(scored: Scored<T>[], runsOnly: boolean): T[] {
	return scored
		.filter((match) => !runsOnly || match.score < SCATTERED)
		.sort((a, b) => a.score - b.score)
		.map((match) => match.item);
}