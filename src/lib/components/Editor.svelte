<script lang="ts">
	import { javascript } from '@codemirror/lang-javascript';
	import { json, jsonParseLinter } from '@codemirror/lang-json';
	import { linter, lintGutter } from '@codemirror/lint';
	import { Compartment, EditorState, Prec } from '@codemirror/state';
	import { oneDark } from '@codemirror/theme-one-dark';
	import { EditorView, placeholder as placeholderExt } from '@codemirror/view';
	import { basicSetup } from 'codemirror';
	import { untrack } from 'svelte';
	import { editorFont, type EditorScope } from '$lib/editor.svelte';
	import { theme } from '$lib/theme.svelte';

	interface Props {
		value?: string;
		readonly?: boolean;
		/** JSON gets highlighting, folding and inline parse errors. */
		language?: 'json' | 'text' | 'typescript';
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

	const editable = new Compartment();
	const languageConf = new Compartment();
	const themeConf = new Compartment();
	const fontConf = new Compartment();

	const parseJson = jsonParseLinter();

	function languageExtensions(lang: 'json' | 'text' | 'typescript') {
		if (lang === 'typescript') return [javascript({ typescript: true })];
		if (lang !== 'json') return [];
		return [
			json(),
			// An empty document is not a JSON error — it's a body you haven't
			// written yet, or a response with no content. Reporting "Unexpected
			// EOF" there is just noise.
			linter((target) => (target.state.doc.length === 0 ? [] : parseJson(target))),
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
						value = update.state.doc.toString();
					})
				]
			})
		});

		view = instance;
		return () => {
			instance.destroy();
			view = undefined;
		};
	});

	// Push external changes in without clobbering what the user is typing — the
	// equality check is what stops this ping-ponging with the update listener.
	$effect(() => {
		const next = value;
		if (!view || next === view.state.doc.toString()) return;
		view.dispatch({
			changes: { from: 0, to: view.state.doc.length, insert: next }
		});
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
