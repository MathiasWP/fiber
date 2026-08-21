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

/** What the schema walk leaves behind. `null` is absent on purpose: it is a value. */
const TYPES = ['string', 'number', 'integer', 'boolean'];

export interface Slot {
	from: number;
	to: number;
}

const WORD = /[A-Za-z]/;

/**
 * Every unfilled field in `text`.
 *
 * A bare word cannot appear in valid JSON, so anything outside a string that
 * spells a type name is a placeholder. The scan tracks quoting for exactly that
 * reason: in `"note": "a number goes here"` the word is content, not a gap.
 */
export function slots(text: string): Slot[] {
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

	view.dispatch({
		selection: EditorSelection.single(next.from, next.to),
		scrollIntoView: true
	});
	return true;
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
 * A comma is something you would type anyway, so this costs nothing when you
 * don't want it, and does nothing at all once the body has no gaps left.
 */
const advance = EditorView.updateListener.of((update) => {
	if (!update.docChanged) return;

	let closed = false;
	for (const tr of update.transactions) {
		if (!tr.isUserEvent('input.type')) continue;
		tr.changes.iterChanges((_fromA, _toA, _fromB, _toB, inserted) => {
			if (inserted.toString() === ',') closed = true;
		});
	}
	if (!closed) return;

	const cursor = update.state.selection.main;
	const next = slots(update.state.doc.toString()).find((slot) => slot.from >= cursor.to);
	if (!next) return;

	// Out of the update cycle: dispatching during one is not allowed.
	queueMicrotask(() =>
		update.view.dispatch({
			selection: EditorSelection.single(next.from, next.to),
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
