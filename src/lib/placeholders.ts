import { EditorSelection, Prec, StateField, type Extension } from '@codemirror/state';
import { Decoration, EditorView, keymap, type DecorationSet } from '@codemirror/view';

/**
 * Unfilled fields in a generated request body.
 *
 * A body built from an OpenAPI schema comes back with the *names of types* in
 * place of values — `"offset": number` rather than `"offset": 0`. An empty
 * value tells you nothing about what belongs there, and it looks finished when
 * it isn't; a type name tells you both. The cost is that the body is not valid
 * JSON until every one has been replaced, which is the intent.
 *
 * This finds them, marks them, and moves between them.
 */

/**
 * What the schema walk leaves behind. `null` is absent on purpose: it is a
 * value. `unknown` is what the importer writes where the schema said nothing
 * it could read — still a gap, so still tabbable.
 */
const TYPES = ['string', 'number', 'integer', 'boolean', 'unknown'];

export interface Slot {
	from: number;
	to: number;
}

const WORD = /[A-Za-z]/;

/**
 * Above this, scanning for type-name placeholders is more expensive than the
 * placeholders are worth. Generated OpenAPI skeletons sit well under it;
 * a hand-written body this large has none, and walking every character of it
 * on each keystroke is a freeze to highlight nothing.
 */
const SCAN_LIMIT = 64 * 1024;

/**
 * Every unfilled field in `text`.
 *
 * A bare word cannot appear in valid JSON, so anything outside a string that
 * spells a type name is a placeholder. The scan tracks quoting for exactly that
 * reason: in `"note": "a number goes here"` the word is content, not a gap.
 */
export function slots(text: string): Slot[] {
	if (text.length > SCAN_LIMIT) return [];
	const found: Slot[] = [];
	let quoted = false;

	for (let i = 0; i < text.length; ) {
		const char = text[i];

		if (quoted) {
			// A backslash escapes whatever follows, including a quote.
			if (char === '\\') i += 2;
			else {
				if (char === '"') quoted = false;
				i++;
			}
			continue;
		}

		if (char === '"') {
			quoted = true;
			i++;
			continue;
		}

		if (WORD.test(char)) {
			let end = i;
			while (end < text.length && WORD.test(text[end])) end++;
			if (TYPES.includes(text.slice(i, end))) found.push({ from: i, to: end });
			i = end;
			continue;
		}

		i++;
	}

	return found;
}

const mark = Decoration.mark({ class: 'cm-slot' });

function decorate(text: string): DecorationSet {
	return Decoration.set(slots(text).map((slot) => mark.range(slot.from, slot.to)));
}

const marks = StateField.define<DecorationSet>({
	create: (state) => decorate(state.doc.toString()),
	update: (current, tr) => (tr.docChanged ? decorate(tr.state.doc.toString()) : current),
	provide: (field) => EditorView.decorations.from(field)
});

/** Selects the next unfilled field, so typing replaces it. */
function jump(view: EditorView, forward: boolean): boolean {
	const found = slots(view.state.doc.toString());
	if (found.length === 0) return false;

	const cursor = view.state.selection.main;
	const next = forward
		? (found.find((slot) => slot.from >= cursor.to) ?? found[0])
		: (found.findLast((slot) => slot.to <= cursor.from) ?? found[found.length - 1]);

	view.dispatch({ selection: select(next), scrollIntoView: true });
	return true;
}

/**
 * The whole field, with the caret at the *start* of it.
 *
 * Backwards on purpose. A forward selection leaves the caret sitting after the
 * last letter, and the field's own tint covers the selection highlight — so an
 * unfilled field you have just tabbed to looks exactly like one you have not,
 * with a caret parked next to it. Either way the next thing you type replaces
 * the lot, but only one of them looks like it will.
 */
function select(slot: Slot) {
	return EditorSelection.single(slot.to, slot.from);
}

/**
 * Whether `index` sits inside a string, by the same reading [`slots`] uses.
 *
 * The one thing the advance below has to know. A comma in `"Ada, Lovelace"` is
 * part of a name; a comma after it is punctuation.
 */
function quotedAt(text: string, index: number): boolean {
	let quoted = false;
	for (let i = 0; i < index && i < text.length; ) {
		const char = text[i];
		if (quoted) {
			if (char === '\\') i += 2;
			else {
				if (char === '"') quoted = false;
				i++;
			}
			continue;
		}
		if (char === '"') quoted = true;
		i++;
	}
	return quoted;
}

/** The nearest non-space character either side of `index`, or `''`. */
function neighbour(text: string, index: number, step: -1 | 1): string {
	for (let i = index + step; i >= 0 && i < text.length; i += step) {
		if (!/\s/.test(text[i])) return text[i];
	}
	return '';
}

/**
 * Moves on once a value is finished, the way filling in a form does.
 *
 * The comma is the signal, and only the comma. A closing quote looks like the
 * same thing and isn't: typing `"` over a selected field makes CodeMirror wrap
 * it rather than replace it — which is what you want, since it leaves the
 * placeholder selected inside fresh quotes for you to type over — but it means
 * the *opening* quote and the closing one are indistinguishable here. Advancing
 * on either one moved the cursor away mid-word.
 *
 * Two commas are not the signal, and this is where it went wrong. A comma
 * inside a string value is part of the value: typing `"Ada, Lovelace"` used to
 * jump to the next field halfway through the name and type the rest of it over
 * that field's placeholder, which quietly destroyed both. And a body generated
 * from a schema already has its separators, so the comma you type to mean
 * "done" landed next to one that was already there and left `1,,`.
 *
 * So: only outside a string, and the typed comma is dropped when it only
 * duplicates one already in the body. What you meant by it still happens.
 */
const advance = EditorView.updateListener.of((update) => {
	if (!update.docChanged) return;

	let at: number | null = null;
	for (const tr of update.transactions) {
		if (!tr.isUserEvent('input.type')) continue;
		tr.changes.iterChanges((_fromA, _toA, _fromB, toB, inserted) => {
			// One character, so the comma is the last position it occupies.
			if (inserted.toString() === ',') at = toB - 1;
		});
	}
	if (at === null) return;
	// A const, so the closure below keeps the narrowing.
	const comma: number = at;

	const text = update.state.doc.toString();
	const cursor = update.state.selection.main;

	if (quotedAt(text, comma)) {
		// Inside a value a comma is part of the value, and there is no reading
		// of the keystroke that says otherwise. `"Ada,` could be someone done
		// with a name or halfway through `"Ada, Lovelace"`, and guessing wrong
		// on the second one used to jump away and type the rest of the name
		// over the next field's placeholder. Typing the closing quote is the
		// unambiguous way to say "done" — the caret steps over the auto-closed
		// one, and the comma after it lands here as an ordinary separator.
		return;
	}

	// A double comma is never valid JSON, so a comma next to a comma is always
	// the one just typed being redundant — whichever side it landed on. This is
	// what a generated body does to you: it already has its separators, so the
	// comma you type to mean "done" left `1,,`.
	const duplicate = neighbour(text, comma, 1) === ',' || neighbour(text, comma, -1) === ',';
	const cut = duplicate ? comma : null;
	const from = duplicate ? comma : cursor.to;

	const remaining = cut === null ? text : text.slice(0, cut) + text.slice(cut + 1);
	// Everything after the cut has shifted left by one.
	const start = cut !== null && cut < from ? from - 1 : from;
	const next = slots(remaining).find((slot) => slot.from >= start);

	// Nothing to drop and nowhere to go: leave the comma where the user put it.
	if (cut === null && !next) return;

	// Out of the update cycle: dispatching during one is not allowed.
	queueMicrotask(() =>
		update.view.dispatch({
			...(cut === null ? {} : { changes: { from: cut, to: cut + 1 } }),
			...(next ? { selection: select(next) } : {}),
			scrollIntoView: true
		})
	);
});

export const placeholders: Extension = [
	marks,
	advance,
	// Above the default Tab binding, which indents. Falls through when the body
	// has no gaps, so Tab still indents in an ordinary body.
	Prec.high(
		keymap.of([
			{
				key: 'Tab',
				run: (view) => jump(view, true),
				shift: (view) => jump(view, false)
			}
		])
	),
	EditorView.baseTheme({
		'.cm-slot': {
			color: 'rgb(var(--c-accent))',
			backgroundColor: 'color-mix(in srgb, rgb(var(--c-accent)) 12%, transparent)',
			borderRadius: '2px',
			// Dotted rather than solid: a gap to fill, not an error to fix.
			borderBottom: '1px dotted rgb(var(--c-accent))'
		}
	})
];
