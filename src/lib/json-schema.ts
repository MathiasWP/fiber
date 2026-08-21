/** A small, deliberately useful subset of JSON Schema for request editing. */
interface Schema {
	type?: string | string[];
	nullable?: boolean;
	enum?: unknown[];
	const?: unknown;
	required?: string[];
	properties?: Record<string, unknown>;
	items?: unknown;
	additionalProperties?: boolean | unknown;
	allOf?: unknown[];
	anyOf?: unknown[];
	oneOf?: unknown[];
}

function isSchema(value: unknown): value is Schema {
	return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function valueKind(value: unknown): string {
	if (value === null) return 'null';
	if (Array.isArray(value)) return 'array';
	return typeof value;
}

function acceptsType(value: unknown, type: string): boolean {
	switch (type) {
		case 'null':
			return value === null;
		case 'boolean':
			return typeof value === 'boolean';
		case 'string':
			return typeof value === 'string';
		case 'number':
			return typeof value === 'number';
		case 'integer':
			return typeof value === 'number' && Number.isInteger(value);
		case 'array':
			return Array.isArray(value);
		case 'object':
			return value !== null && typeof value === 'object' && !Array.isArray(value);
		default:
			return true;
	}
}

function childPath(parent: string, key: string): string {
	return /^[A-Za-z_$][\w$]*$/.test(key) ? `${parent}.${key}` : `${parent}[${JSON.stringify(key)}]`;
}

function validate(value: unknown, schema: unknown, path: string): string[] {
	if (!isSchema(schema)) return [];

	const errors: string[] = [];
	const types = schema.type ? (Array.isArray(schema.type) ? schema.type : [schema.type]) : [];
	if (schema.nullable && !types.includes('null')) types.push('null');
	if (types.length && !types.some((type) => acceptsType(value, type))) {
		errors.push(`${path} must be ${types.join(' or ')}, not ${valueKind(value)}.`);
		return errors;
	}

	if (schema.const !== undefined && JSON.stringify(value) !== JSON.stringify(schema.const)) {
		errors.push(`${path} must equal ${JSON.stringify(schema.const)}.`);
	}
	if (schema.enum && !schema.enum.some((option) => JSON.stringify(option) === JSON.stringify(value))) {
		errors.push(`${path} must be one of: ${schema.enum.map((option) => JSON.stringify(option)).join(', ')}.`);
	}

	for (const part of schema.allOf ?? []) errors.push(...validate(value, part, path));
	if (schema.anyOf && !schema.anyOf.some((part) => validate(value, part, path).length === 0)) {
		errors.push(`${path} does not match any allowed schema.`);
	}
	if (schema.oneOf) {
		const hits = schema.oneOf.filter((part) => validate(value, part, path).length === 0).length;
		if (hits !== 1) {
			errors.push(`${path} must match exactly one allowed schema.`);
		}
	}

	if (!isSchema(value) || Array.isArray(value)) {
		if (Array.isArray(value) && schema.items) {
			for (const [index, item] of value.entries()) errors.push(...validate(item, schema.items, `${path}[${index}]`));
		}
		return errors;
	}

	for (const key of schema.required ?? []) {
		if (!Object.hasOwn(value, key)) errors.push(`${childPath(path, key)} is required.`);
	}
	for (const [key, item] of Object.entries(value)) {
		const property = schema.properties?.[key];
		if (property) {
			errors.push(...validate(item, property, childPath(path, key)));
		} else if (schema.additionalProperties === false) {
			errors.push(`${childPath(path, key)} is not allowed.`);
		} else if (isSchema(schema.additionalProperties)) {
			errors.push(...validate(item, schema.additionalProperties, childPath(path, key)));
		}
	}
	return errors;
}

/**
 * Returns actionable schema errors for a complete JSON body. JSON syntax stays
 * the editor's concern: reporting a schema mismatch before the document parses
 * would only flicker errors while someone is in the middle of typing.
 */
export function validateJsonBody(schema: unknown | null, text: string): string[] {
	if (!schema || !text.trim()) return [];
	try {
		return validate(JSON.parse(text), schema, '$');
	} catch {
		return [];
	}
}
