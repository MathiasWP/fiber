import { expect, test } from '@playwright/test';
import { validateJsonBody } from '../../src/lib/json-schema';

test.describe('validateJsonBody', () => {
	test('reports a type mismatch', () => {
		expect(validateJsonBody({ type: 'boolean' }, 'null')).toEqual([
			'$ must be boolean, not null.'
		]);
	});

	test('requires listed properties', () => {
		expect(
			validateJsonBody({ type: 'object', required: ['enabled'] }, '{ "other": true }')
		).toEqual(['$.enabled is required.']);
	});

	test('treats oneOf as exactly one match, not anyOf', () => {
		const schema = { oneOf: [{ type: 'number' }, { type: 'integer' }] };
		expect(validateJsonBody(schema, '1')).toEqual(['$ must match exactly one allowed schema.']);
		expect(validateJsonBody(schema, '1.5')).toEqual([]);
	});

	test('anyOf still accepts the first matching branch', () => {
		const schema = { anyOf: [{ type: 'number' }, { type: 'integer' }] };
		expect(validateJsonBody(schema, '1')).toEqual([]);
	});

	test('allOf applies every branch', () => {
		const schema = {
			allOf: [{ type: 'object' }, { required: ['id'] }, { properties: { id: { type: 'string' } } }]
		};
		expect(validateJsonBody(schema, '{ "id": 1 }')).toEqual(['$.id must be string, not number.']);
		expect(validateJsonBody(schema, '{}')).toEqual(['$.id is required.']);
	});

	test('enum and const reject other values', () => {
		expect(validateJsonBody({ enum: ['a', 'b'] }, '"c"')).toEqual(['$ must be one of: "a", "b".']);
		expect(validateJsonBody({ const: 3 }, '2')).toEqual(['$ must equal 3.']);
	});

	test('additionalProperties can forbid or check extras', () => {
		expect(
			validateJsonBody({ type: 'object', additionalProperties: false }, '{ "x": 1 }')
		).toEqual(['$.x is not allowed.']);
		expect(
			validateJsonBody(
				{ type: 'object', additionalProperties: { type: 'number' } },
				'{ "x": "no" }'
			)
		).toEqual(['$.x must be number, not string.']);
	});

	test('array items are checked by index', () => {
		expect(validateJsonBody({ type: 'array', items: { type: 'number' } }, '[1, "x"]')).toEqual([
			'$[1] must be number, not string.'
		]);
	});

	test('nullable accepts null without dropping the other type', () => {
		expect(validateJsonBody({ type: 'string', nullable: true }, 'null')).toEqual([]);
		expect(validateJsonBody({ type: ['string', 'null'] }, 'null')).toEqual([]);
		expect(validateJsonBody({ type: 'string', nullable: true }, '1')).toEqual([
			'$ must be string or null, not number.'
		]);
	});

	test('a key that is not a JS identifier is addressed in brackets', () => {
		expect(
			validateJsonBody(
				{ type: 'object', properties: { 'content-type': { type: 'number' } } },
				'{ "content-type": true }'
			)
		).toEqual(['$["content-type"] must be number, not boolean.']);
	});

	/**
	 * Pinned because it surprises everyone, including me: `additionalProperties`
	 * is scoped to the `properties` beside it and deliberately does not see what
	 * an `allOf` branch introduces. So this really is two disallowed fields, and
	 * a spec written this way rejects its own documents. Asserted so nobody
	 * later "fixes" it into a leniency the standard does not have.
	 */
	test('additionalProperties does not see an allOf branch, per the spec', () => {
		const schema = {
			allOf: [
				{ type: 'object', properties: { id: { type: 'string' } } },
				{ type: 'object', properties: { name: { type: 'string' } } }
			],
			type: 'object',
			additionalProperties: false
		};
		expect(validateJsonBody(schema, '{ "id": "a", "name": "b" }')).toEqual([
			'$.id is not allowed.',
			'$.name is not allowed.'
		]);
	});

	/** `$ref` inside the document, which the walker ignored outright. */
	test('a local $ref is followed', () => {
		const schema = {
			type: 'object',
			properties: { child: { $ref: '#/$defs/leaf' } },
			$defs: { leaf: { type: 'number' } }
		};
		expect(validateJsonBody(schema, '{ "child": "no" }')).toEqual([
			'$.child must be number, not string.'
		]);
	});

	/** Keywords the subset never covered at all. */
	test('constraints beyond the old subset are enforced', () => {
		expect(validateJsonBody({ type: 'integer', minimum: 1 }, '0')).toEqual([
			'$ must be >= 1.'
		]);
		expect(validateJsonBody({ type: 'array', items: { type: 'number' }, uniqueItems: true }, '[1, 1]')).not.toEqual([]);
		expect(validateJsonBody({ type: 'string', pattern: '^a' }, '"b"')).not.toEqual([]);
	});

	/**
	 * A real 3.1 document reaches us with `"type": "undefined"` 310 times. Ajv
	 * throws on a type that does not exist, which would cost the schema all of
	 * its linting rather than just that field's.
	 */
	test('an invented type does not disable the rest of the schema', () => {
		const schema = {
			type: 'object',
			required: ['id'],
			properties: { ignored: { type: 'undefined' }, id: { type: 'string' } }
		};
		expect(validateJsonBody(schema, '{ "ignored": 1, "id": 2 }')).toEqual([
			'$.id must be string, not number.'
		]);
		expect(validateJsonBody(schema, '{ "ignored": "anything", "id": "ok" }')).toEqual([]);
	});

	/** OpenAPI 3.1's literal union, which is what `const` is for. */
	test('a choice of consts reports the composition, not each branch', () => {
		const schema = {
			anyOf: [
				{ type: 'string', const: 'once' },
				{ type: 'string', const: 'always' }
			]
		};
		expect(validateJsonBody(schema, '"once"')).toEqual([]);
		expect(validateJsonBody(schema, '"twice"')).toEqual([
			'$ does not match any allowed schema.'
		]);
	});

	/** A property literally called "type" is a name, not a keyword. */
	test('a property named type is not read as one', () => {
		const schema = {
			type: 'object',
			properties: { type: { type: 'string' } }
		};
		expect(validateJsonBody(schema, '{ "type": 1 }')).toEqual([
			'$.type must be string, not number.'
		]);
	});

	test('invalid or empty JSON is not a schema error', () => {
		expect(validateJsonBody({ type: 'object' }, '')).toEqual([]);
		expect(validateJsonBody({ type: 'object' }, '{')).toEqual([]);
		expect(validateJsonBody(null, '{}')).toEqual([]);
	});

	test('oversized bodies are not parsed', () => {
		const huge = `{"x":"${'a'.repeat(2 * 1024 * 1024)}"}`;
		expect(validateJsonBody({ type: 'object' }, huge)).toEqual([]);
	});
});
