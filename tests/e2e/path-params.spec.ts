import { expect, test } from '@playwright/test';
import { applyPathParams, parseQuery, pathParamNames, withQuery } from '../../src/lib/api';

test.describe('path params', () => {
	test('names come from placeholders in the path, not the query', () => {
		expect(pathParamNames('/pet/{petId}?pretty=1')).toEqual(['petId']);
		expect(pathParamNames('/store/{orderId}/items/{itemId}')).toEqual(['orderId', 'itemId']);
	});

	test('empty values stay visible as placeholders', () => {
		expect(applyPathParams('/pet/{petId}', [{ name: 'petId', value: '' }])).toBe('/pet/{petId}');
		expect(applyPathParams('/pet/{petId}', [])).toBe('/pet/{petId}');
	});

	test('a slash in a value does not become another path component', () => {
		expect(applyPathParams('/pet/{petId}', [{ name: 'petId', value: 'a/b' }])).toBe('/pet/a%2Fb');
	});

	test('query editing preserves URL fragments in the right position', () => {
		expect(parseQuery('/users?x=1#details')).toEqual([{ name: 'x', value: '1' }]);
		expect(withQuery('/users?x=1#details', [{ name: 'x', value: '2' }])).toBe(
			'/users?x=2#details'
		);
		expect(withQuery('/users#details', [{ name: 'added', value: 'yes' }])).toBe(
			'/users?added=yes#details'
		);
	});
});
