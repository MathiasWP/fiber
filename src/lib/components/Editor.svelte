<script lang="ts">
	import { json, jsonParseLinter } from '@codemirror/lang-json';
	import { linter, lintGutter } from '@codemirror/lint';
	import { Annotation, Compartment, EditorState, Prec } from '@codemirror/state';
	import { oneDark } from '@codemirror/theme-one-dark';
	import { EditorView, placeholder as placeholderExt } from '@codemirror/view';
	import { JSON_TOOLING_LIMIT } from '$lib/api';
	import { placeholders } from '$lib/placeholders';
	import { basicSetup } from 'codemirror';
	import { untrack } from 'svelte';
	import { editorFont, type EditorScope } from '$lib/editor.svelte';
	import { theme } from '$lib/theme.svelte';

	interface Props {
		value?: string;
		readonly?: boolean;
		/** JSON gets highlighting, folding and inline parse errors. */
		language?: 'json' | 'text';
		placeholder?: string;
		/** Which of the two text sizes this editor follows. */
		scope?: EditorScope;
	}

	let {
		value = $bindable(''),
		readonly = false,
		language = 'json',
		placeholder = '',
		scope = 'request'
	}: Props = $props();

	let host = $state<HTMLDivElement>();
	let view: EditorView | undefined;

	/** Marks a transaction this component dispatched itself, from the `value` prop. */
	const fromProp = Annotation.define<boolean>();

	/**
	 * The document's current text, as a string this component already holds.
	 *
	 * Every path that changes the doc keeps it current — the update listener for
	 * typing and `format`, the sync effect for prop changes — so the effect can
	 * ask "did `value` merely grow?" against a plain string instead of calling
	 * `doc.toString()`, which for a streaming 32 MB body was a fresh full copy
	 * per animation frame.
	 */
	let synced = '';

	const editable = new Compartment();
	const languageConf = new Compartment();
	const themeConf = new Compartment();
	const fontConf = new Compartment();

	const parseJson = jsonParseLinter();

	function languageExtensions(lang: 'json' | 'text') {
		if (lang !== 'json') return [];
		return [
			json(),
			// An empty document is not a JSON error — it's a body you haven't
			// written yet, or a response with no content. Reporting "Unexpected
			// EOF" there is just noise. And an oversized one gets no lint pass at
			// all: the linter is a whole JSON.parse of the document, run on top
			// of the parse the language mode already did, which for a body of
			// tens of megabytes is seconds of frozen UI to underline nothing.
			linter((target) =>
				target.state.doc.length === 0 || target.state.doc.length > JSON_TOOLING_LIMIT
					? []
					: parseJson(target)
			),
			lintGutter()
		];
	}

	/** Its own compartment so the size can change without rebuilding the view. */
	function fontTheme(size: number) {
		return EditorView.theme({ '&': { fontSize: `${size}px` } });
	}

	$effect(() => {
		if (!host) return;

		const instance = new EditorView({
			parent: host,
			state: EditorState.create({
				doc: untrack(() => value),
				extensions: [
					basicSetup,
					themeConf.of(untrack(() => (theme.resolved === 'dark' ? oneDark : []))),
					languageConf.of(languageExtensions(untrack(() => language))),
					// Editable, but read-only: `EditorView.editable.of(false)` sets
					// contenteditable=false, which makes the editor unfocusable —
					// and an unfocused editor means ⌘A selects the whole app
					// instead of the response. `EditorState.readOnly` blocks edits
					// while keeping focus and selection.
					editable.of(EditorView.editable.of(true)),
					EditorState.readOnly.of(untrack(() => readonly)),
					EditorView.lineWrapping,
					placeholderExt(untrack(() => placeholder)),
					// Only where something can be filled in. A response has no gaps
					// to tab between, and its Tab should stay Tab.
					untrack(() => readonly) ? [] : placeholders,
					// Prec.high because oneDark styles `.cm-panels` too, and which of two
					// themes wins otherwise depends on extension order — a detail that
					// would quietly break the day someone reorders this array.
					Prec.high(
						EditorView.theme({
							'&': { height: '100%' },
							'.cm-scroller': { fontFamily: 'var(--font-mono)' },
							'&.cm-focused': { outline: 'none' },

							// ⌘F opens CodeMirror's own search panel, and it ships with
							// none of this: a bare text input, three default buttons and
							// full-size system checkboxes, in whatever the platform thinks
							// those look like. Dressed here to match everything around it.
							'.cm-panels': {
								backgroundColor: 'rgb(var(--c-panel))',
								color: 'rgb(var(--c-text))'
							},
							'.cm-panels.cm-panels-bottom': { borderTop: '1px solid rgb(var(--c-border))' },
							'.cm-panels.cm-panels-top': { borderBottom: '1px solid rgb(var(--c-border))' },
							'.cm-panel.cm-search': {
								display: 'flex',
								flexWrap: 'wrap',
								alignItems: 'center',
								gap: '6px',
								padding: '6px 8px',
								fontFamily: 'inherit',
								fontSize: '11px'
							},
							'.cm-panel.cm-search label': {
								display: 'inline-flex',
								alignItems: 'center',
								gap: '4px',
								fontSize: '11px',
								color: 'rgb(var(--c-muted))'
							},
							// The default is a 16px system checkbox, which towers over
							// 11px labels.
							'.cm-panel.cm-search input[type=checkbox]': {
								width: '11px',
								height: '11px',
								accentColor: 'rgb(var(--c-accent))'
							},
							'.cm-textfield': {
								backgroundColor: 'rgb(var(--c-raised))',
								border: '1px solid rgb(var(--c-border))',
								borderRadius: '4px',
								padding: '3px 6px',
								fontSize: '11px',
								fontFamily: 'var(--font-mono)',
								color: 'rgb(var(--c-text))'
							},
							'.cm-textfield:focus': {
								outline: 'none',
								borderColor: 'rgb(var(--c-accent))'
							},
							'.cm-button': {
								backgroundImage: 'none',
								backgroundColor: 'rgb(var(--c-raised))',
								border: '1px solid rgb(var(--c-border))',
								borderRadius: '4px',
								padding: '3px 8px',
								fontSize: '11px',
								color: 'rgb(var(--c-text))'
							},
							'.cm-button:hover': { backgroundColor: 'rgb(var(--c-border))' },
							'.cm-button:active': { backgroundImage: 'none' },
							'.cm-panel.cm-search [name=close]': {
								position: 'static',
								marginLeft: 'auto',
								padding: '0 4px',
								fontSize: '14px',
								lineHeight: '1',
								color: 'rgb(var(--c-muted))',
								cursor: 'default'
							},
							'.cm-panel.cm-search [name=close]:hover': { color: 'rgb(var(--c-text))' }
						})
					),
					fontConf.of(fontTheme(untrack(() => editorFont.size(scope)))),
					EditorView.updateListener.of((update) => {
						if (!update.docChanged) return;
						// A change this component dispatched from the prop: `value`
						// already says exactly this, so echoing it back would only
						// materialize the whole document — per frame, when streaming.
						if (update.transactions.some((tr) => tr.annotation(fromProp))) return;
						synced = update.state.doc.toString();
						value = synced;
					})
				]
			})
		});

		view = instance;
		synced = untrack(() => value);
		return () => {
			instance.destroy();
			view = undefined;
		};
	});

	/**
	 * Whether `next` merely extends `prior` — the streaming case, where each
	 * frame's value is the last one plus a chunk.
	 *
	 * Exact for anything small. Past a quarter megabyte the prefix compare is
	 * itself a full walk of the old value every frame — the O(n²) this exists to
	 * remove — so it samples instead: the head, and the stretch just before the
	 * append point. A wholesale replacement that keeps the length growing *and*
	 * matches both samples could fool it, but `prior` here is always the exact
	 * string this component last put in the doc, and streams only ever append.
	 */
	function extendsPrior(next: string, prior: string): boolean {
		if (next.length <= prior.length || prior.length === 0) return false;
		if (prior.length <= 256 * 1024) return next.startsWith(prior);
		const probe = 64;
		return (
			next.slice(0, probe) === prior.slice(0, probe) &&
			next.slice(prior.length - probe, prior.length) === prior.slice(-probe)
		);
	}

	// Push external changes in without clobbering what the user is typing — the
	// `synced` check is what stops this ping-ponging with the update listener.
	// When the new value strictly extends the old one, only the new tail is
	// dispatched: CodeMirror then treats it as an append rather than tearing
	// down and re-inserting the whole document on every streamed chunk.
	$effect(() => {
		const next = value;
		if (!view || next === synced) return;
		if (extendsPrior(next, synced)) {
			view.dispatch({
				changes: { from: synced.length, insert: next.slice(synced.length) },
				annotations: fromProp.of(true)
			});
		} else {
			view.dispatch({
				changes: { from: 0, to: view.state.doc.length, insert: next },
				annotations: fromProp.of(true)
			});
		}
		synced = next;
	});

	$effect(() => {
		view?.dispatch({
			effects: languageConf.reconfigure(languageExtensions(language))
		});
	});

	$effect(() => {
		view?.dispatch({
			effects: themeConf.reconfigure(theme.resolved === 'dark' ? oneDark : [])
		});
	});

	$effect(() => {
		view?.dispatch({ effects: fontConf.reconfigure(fontTheme(editorFont.size(scope))) });
	});

	$effect(() => {
		// Read-only is enforced by the state facet above; the view stays editable
		// so it can hold focus.
		view?.dispatch({
			effects: editable.reconfigure(EditorView.editable.of(true))
		});
	});

	export function format(): void {
		if (!view) return;
		try {
			const pretty = JSON.stringify(JSON.parse(view.state.doc.toString()), null, 2);
			view.dispatch({
				changes: { from: 0, to: view.state.doc.length, insert: pretty }
			});
		} catch {
			// Not valid JSON — the linter is already saying so in the gutter.
		}
	}
</script>

<!-- `focusin` rather than `focus`: the latter doesn't bubble, and the thing
     actually taking focus is CodeMirror's contenteditable inside this div. -->
<div
	bind:this={host}
	class="h-full overflow-hidden"
	onfocusin={() => (editorFont.active = scope)}
></div>
