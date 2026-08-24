// `.js` explicitly: Ajv 8 ships no `exports` map, so Node resolves this path
// literally and an extensionless import fails outside a bundler.
import Ajv2020, { type ErrorObject, type ValidateFunction } from 'ajv/dist/2020.js';

/**
 * Schema linting for the request and response panes, on top of Ajv.
 *
 * This was a hand-written walker over "a deliberately useful subset" of JSON
 * Schema, and the subset was the problem: no `$ref`, no `minimum`, `pattern`,
 * `uniqueItems`, `minLength`, `patternProperties`, `if`/`then`, or
 * `dependentSchemas`. Every one of those silently passed, so a body could be
 * reported clean and still be rejected by the API that published the schema.
 * Ajv is the reference implementation of the thing we were approximating.
 *
 * What stays hand-written is everything that is *not* JSON Schema — the two
 * ways a real OpenAPI document diverges from it, both handled in `normalize`
 * below. Ajv is strict about its own spec, and correctly so; the documents we
 * are handed are not.
 */

/** The only values `type` may hold. Anything else is a generator's invention. */
const JSON_TYPES = new Set([
	'null',
	'boolean',
	'object',
	'array',
	'number',
	'string',
	'integer'
]);

const ajv = new Ajv2020({
	// Every error, not just the first: the pane lists them.
	allErrors: true,
	// OpenAPI schemas carry keywords Ajv doesn't know — `discriminator`, `xml`,
	// `externalDocs`. They are annotations, not assertions, so ignoring them is
	// right; strict mode would make each one fatal instead.
	strict: false,
	// `format` in OpenAPI is a documentation hint as often as a constraint
	// (`format: "int64"`, `format: "uuid"`), and failing a body over one would
	// be noise where the API itself does not care.
	validateFormats: false
});

/**
 * An OpenAPI schema, made into one Ajv will accept.
 *
 * Two divergences, both real and both seen in specs in the wild:
 *
 * - **3.0's `nullable: true`.** 3.0 predates JSON Schema's union types and
 *   spells nullability with its own keyword. 3.1 writes `type: ["string",
 *   "null"]`. Ajv implements 3.1's reading, so the older spelling is folded
 *   into it here rather than silently ignored — ignoring it would report a
 *   legitimate `null` as the wrong type.
 * - **Types that do not exist.** One real 3.1 document reaches us with
 *   `"type": "undefined"` 310 times, plus `emoji`, `icon`, `void` and `http`.
 *   Ajv throws on those at compile time, which would cost the whole document
 *   its linting over a field nobody was going to check anyway. Dropping the
 *   invalid names keeps every valid constraint in the same schema working.
 */
function normalize(value: unknown): unknown {
	if (Array.isArray(value)) return value.map(normalize);
	if (value === null || typeof value !== 'object') return value;

	const source = value as Record<string, unknown>;
	const out: Record<string, unknown> = {};
	for (const [key, nested] of Object.entries(source)) {
		// `properties` and `$defs` hold *names*, which may be anything at all —
		// a property called "type" is a property, not a type. Recurse into their
		// values without treating their keys as keywords.
		out[key] = key === 'properties' || key === '$defs' || key === 'definitions'
			? Object.fromEntries(
					Object.entries((nested ?? {}) as Record<string, unknown>).map(([name, schema]) => [
						name,
						normalize(schema)
					])
				)
			: normalize(nested);
	}

	if ('type' in out) {
		const declared = (Array.isArray(out.type) ? out.type : [out.type]).filter(
			(name): name is string => typeof name === 'string' && JSON_TYPES.has(name)
		);
		if (out.nullable === true && !declared.includes('null')) declared.push('null');

		if (declared.length === 0) {
			// Every name was an invention. Saying nothing about the type still
			// leaves `required`, `enum` and the rest of this schema enforceable.
			delete out.type;
		} else {
			out.type = declared.length === 1 ? declared[0] : declared;
		}
	}

	return out;
}

/**
 * Compiled validators, keyed by the schema they came from.
 *
 * Linting runs on every keystroke and compiling is the expensive half, so the
 * result is held for as long as the schema object is. A `WeakMap` because the
 * key is the schema the loader cache handed us: when the section's cache is
 * replaced, the old schemas and their validators go together.
 *
 * `null` marks a schema Ajv refused outright, so a bad one is diagnosed once
 * rather than on every keystroke.
 */
const compiled = new WeakMap<object, ValidateFunction | null>();

function validatorFor(schema: object): ValidateFunction | null {
	const held = compiled.get(schema);
	if (held !== undefined) return held;

	let built: ValidateFunction | null = null;
	try {
		built = ajv.compile(normalize(schema) as object);
	} catch (error) {
		// A schema this cannot read is not the user's problem to solve, and
		// certainly not one to report against their body. Lint nothing instead.
		console.warn('schema could not be compiled, skipping validation', error);
	}
	compiled.set(schema, built);
	return built;
}

/**
 * `$.items[0].name` — the shape the pane has always shown.
 *
 * Ajv reports JSON Pointer (`/items/0/name`), which is correct and not what
 * anyone reading a JSON body is looking at.
 */
function pointerToPath(pointer: string): string {
	if (!pointer) return '$';
	return pointer
		.split('/')
		.slice(1)
		.reduce((path, raw) => {
			const key = raw.replace(/~1/g, '/').replace(/~0/g, '~');
			if (/^\d+$/.test(key)) return `${path}[${key}]`;
			return /^[A-Za-z_$][\w$]*$/.test(key) ? `${path}.${key}` : `${path}[${JSON.stringify(key)}]`;
		}, '$');
}

/** What a value actually is, for the half of a type error Ajv leaves out. */
function valueKind(value: unknown): string {
	if (value === null) return 'null';
	if (Array.isArray(value)) return 'array';
	if (Number.isInteger(value)) return 'number';
	return typeof value;
}

/** Walks a JSON Pointer into the parsed body, to reach the offending value. */
function valueAtPointer(root: unknown, pointer: string): unknown {
	if (!pointer) return root;
	let current = root;
	for (const raw of pointer.split('/').slice(1)) {
		if (current === null || typeof current !== 'object') return undefined;
		const key = raw.replace(/~1/g, '/').replace(/~0/g, '~');
		current = (current as Record<string, unknown>)[key];
	}
	return current;
}

/** One Ajv error as a sentence, in the voice the pane already used. */
function describe(error: ErrorObject, root: unknown): string {
	const path = pointerToPath(error.instancePath);

	switch (error.keyword) {
		case 'required':
			return `${pointerToPath(`${error.instancePath}/${error.params.missingProperty}`)} is required.`;
		case 'additionalProperties':
			return `${pointerToPath(`${error.instancePath}/${error.params.additionalProperty}`)} is not allowed.`;
		case 'type': {
			const expected = Array.isArray(error.params.type)
				? error.params.type.join(' or ')
				: error.params.type;
			// Ajv names what was wanted; naming what arrived is the other half of
			// what makes the message actionable without going to look.
			return `${path} must be ${expected}, not ${valueKind(valueAtPointer(root, error.instancePath))}.`;
		}
		case 'enum':
			return `${path} must be one of: ${(error.params.allowedValues as unknown[]).map((option) => JSON.stringify(option)).join(', ')}.`;
		case 'const':
			return `${path} must equal ${JSON.stringify(error.params.allowedValue)}.`;
		case 'anyOf':
			return `${path} does not match any allowed schema.`;
		case 'oneOf':
			return `${path} must match exactly one allowed schema.`;
		default:
			return `${path} ${error.message ?? 'is invalid'}.`;
	}
}

/**
 * Returns actionable schema errors for a complete JSON body. JSON syntax stays
 * the editor's concern: reporting a schema mismatch before the document parses
 * would only flicker errors while someone is in the middle of typing.
 */
export function validateJsonBody(schema: unknown | null, text: string): string[] {
	if (!schema || typeof schema !== 'object' || !text.trim()) return [];
	// Same ceiling as `JSON_TOOLING_LIMIT` in api.ts: a synchronous JSON.parse
	// of a multi-megabyte body on a keystroke is a frozen window, and the
	// errors it would produce are not readable at that size anyway.
	if (text.length > 1.5 * 1024 * 1024) return [];

	let instance: unknown;
	try {
		instance = JSON.parse(text);
	} catch {
		return [];
	}

	const validate = validatorFor(schema);
	if (!validate || validate(instance)) return [];

	// Ajv reports a failed branch *and* the composition above it. The branch
	// errors are about a schema the value was never going to match, so they
	// read as contradictions; the composition error is the one worth showing.
	const errors = validate.errors ?? [];
	const composed = new Set(
		errors
			.filter((error) => error.keyword === 'anyOf' || error.keyword === 'oneOf')
			.map((error) => error.instancePath)
	);

	const seen = new Set<string>();
	return errors
		.filter(
			(error) =>
				!composed.has(error.instancePath) ||
				error.keyword === 'anyOf' ||
				error.keyword === 'oneOf'
		)
		.map((error) => describe(error, instance))
		.filter((message) => !seen.has(message) && seen.add(message));
}
