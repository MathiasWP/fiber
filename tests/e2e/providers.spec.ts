import { expect, test } from '@playwright/test';
import { identify } from '../../src/lib/providers';

/**
 * `identify` is pure lookup logic — no page needed, same as `search.spec.ts`
 * and `json-schema.spec.ts`. Covers one rule per provider family, plus the
 * ranking behaviour that picks the most specific match.
 */

test.describe('localStorage rules', () => {
	test('Auth0 SPA SDK key with the access token path scores higher than the key alone', () => {
		const withPath = identify(
			'localStorage',
			'@@auth0spajs@@::abc123::default::openid',
			'body.access_token'
		);
		expect(withPath).toEqual({ provider: 'Auth0', weight: 50 });

		const keyOnly = identify('localStorage', 'auth0.abc123.access_token', '');
		expect(keyOnly).toEqual({ provider: 'Auth0', weight: 25 });
	});

	test('Supabase auth token key', () => {
		expect(identify('localStorage', 'sb-myproject-auth-token', 'access_token')).toEqual({
			provider: 'Supabase',
			weight: 50
		});
		expect(identify('localStorage', 'sb-myproject-auth-token', '')).toEqual({
			provider: 'Supabase',
			weight: 25
		});
	});

	test('Firebase authUser key with the token manager path', () => {
		expect(
			identify(
				'localStorage',
				'firebase:authUser:AIzaSyABC:[DEFAULT]',
				'stsTokenManager.accessToken'
			)
		).toEqual({ provider: 'Firebase', weight: 50 });
	});

	test('Firebase without the token path is not matched at all', () => {
		// There is no path-less Firebase localStorage rule, unlike the others.
		expect(identify('localStorage', 'firebase:authUser:AIzaSyABC:[DEFAULT]', '')).toBeNull();
	});

	test('MSAL access-token key, with and without the secret path', () => {
		const key = 'uid.env-accesstoken-clientid--scope';
		expect(identify('localStorage', key, 'secret')).toEqual({
			provider: 'Microsoft (MSAL)',
			weight: 50
		});
		expect(identify('localStorage', key, '')).toEqual({
			provider: 'Microsoft (MSAL)',
			weight: 25
		});
	});

	test('MSAL also matches the msal. prefix directly', () => {
		expect(identify('localStorage', 'msal.account.keys', '')).toEqual({
			provider: 'Microsoft (MSAL)',
			weight: 25
		});
	});

	test('AWS Cognito access and id tokens', () => {
		const key = 'CognitoIdentityServiceProvider.clientid.username.accessToken';
		expect(identify('localStorage', key, '')).toEqual({ provider: 'AWS Cognito', weight: 45 });
	});

	test('Cognito refreshToken is not one of the recognised suffixes', () => {
		const key = 'CognitoIdentityServiceProvider.clientid.username.refreshToken';
		expect(identify('localStorage', key, '')).toBeNull();
	});

	test('Okta token storage, with and without the access-token path', () => {
		expect(identify('localStorage', 'okta-token-storage', 'accessToken.accessToken')).toEqual({
			provider: 'Okta',
			weight: 50
		});
		expect(identify('localStorage', 'okta-token-storage', '')).toEqual({
			provider: 'Okta',
			weight: 25
		});
	});

	test('Clerk, Keycloak, SuperTokens and Stytch localStorage keys', () => {
		expect(identify('localStorage', '__clerk_client_jwt', '')).toEqual({
			provider: 'Clerk',
			weight: 45
		});
		expect(identify('localStorage', 'kc-access', '')).toEqual({ provider: 'Keycloak', weight: 40 });
		expect(identify('localStorage', 'kc-token', '')).toEqual({ provider: 'Keycloak', weight: 40 });
		expect(identify('localStorage', 'st-access-token', '')).toEqual({
			provider: 'SuperTokens',
			weight: 40
		});
		// Matching is case-insensitive here, unlike most other rules.
		expect(identify('localStorage', 'ST-ACCESS-TOKEN', '')).toEqual({
			provider: 'SuperTokens',
			weight: 40
		});
		expect(identify('localStorage', 'stytch_session_jwt', '')).toEqual({
			provider: 'Stytch',
			weight: 40
		});
	});
});

test.describe('cookie rules', () => {
	test('Clerk session vs. client cookie', () => {
		expect(identify('cookie', '__session', '')).toEqual({ provider: 'Clerk', weight: 45 });
		expect(identify('cookie', '__client', '')).toEqual({ provider: 'Clerk', weight: 30 });
	});

	test('Auth.js / NextAuth, plain and secure-prefixed, and the authjs rename', () => {
		expect(identify('cookie', 'next-auth.session-token', '')).toEqual({
			provider: 'Auth.js / NextAuth',
			weight: 45
		});
		expect(identify('cookie', '__Secure-next-auth.session-token', '')).toEqual({
			provider: 'Auth.js / NextAuth',
			weight: 45
		});
		expect(identify('cookie', 'authjs.session-token', '')).toEqual({
			provider: 'Auth.js / NextAuth',
			weight: 45
		});
	});

	test('Auth0 appSession cookie', () => {
		expect(identify('cookie', 'appSession', '')).toEqual({ provider: 'Auth0', weight: 45 });
	});

	test('Better Auth session token, default prefix, renamed prefix, and __Secure-/__Host- variants', () => {
		expect(identify('cookie', 'better-auth.session_token', '')).toEqual({
			provider: 'Better Auth',
			weight: 45
		});
		expect(identify('cookie', '__Secure-auth-staging.session_token', '')).toEqual({
			provider: 'Better Auth',
			weight: 45
		});
		expect(identify('cookie', '__Host-session_token', '')).toEqual({
			provider: 'Better Auth',
			weight: 45
		});
	});

	test('Better Auth session_data scores lower than session_token', () => {
		expect(identify('cookie', 'better-auth.session_data', '')).toEqual({
			provider: 'Better Auth',
			weight: 30
		});
	});

	test('Keycloak, SuperTokens, Ory, Stytch, Okta, Descope and WorkOS cookies', () => {
		expect(identify('cookie', 'KEYCLOAK_IDENTITY', '')).toEqual({ provider: 'Keycloak', weight: 40 });
		expect(identify('cookie', 'KEYCLOAK_SESSION', '')).toEqual({ provider: 'Keycloak', weight: 40 });
		expect(identify('cookie', 'kc-access', '')).toEqual({ provider: 'Keycloak', weight: 40 });
		expect(identify('cookie', 'sAccessToken', '')).toEqual({ provider: 'SuperTokens', weight: 40 });
		expect(identify('cookie', 'ory_kratos_session', '')).toEqual({ provider: 'Ory', weight: 40 });
		expect(identify('cookie', 'ory_session', '')).toEqual({ provider: 'Ory', weight: 40 });
		expect(identify('cookie', 'stytch_session', '')).toEqual({ provider: 'Stytch', weight: 40 });
		expect(identify('cookie', 'stytch_session_jwt', '')).toEqual({ provider: 'Stytch', weight: 40 });
		expect(identify('cookie', 'sid', '')).toEqual({ provider: 'Okta', weight: 30 });
		expect(identify('cookie', 'idx', '')).toEqual({ provider: 'Okta', weight: 30 });
		expect(identify('cookie', 'DS', '')).toEqual({ provider: 'Descope', weight: 30 });
		expect(identify('cookie', 'DSR', '')).toEqual({ provider: 'Descope', weight: 30 });
		expect(identify('cookie', 'wos-session', '')).toEqual({ provider: 'WorkOS', weight: 40 });
	});

	test('framework session cookies: Django, Laravel, PHP, Java, Rails, Express, ASP.NET', () => {
		expect(identify('cookie', 'sessionid', '')).toEqual({ provider: 'Django', weight: 30 });
		expect(identify('cookie', 'laravel_session', '')).toEqual({ provider: 'Laravel', weight: 30 });
		expect(identify('cookie', 'PHPSESSID', '')).toEqual({ provider: 'PHP', weight: 30 });
		expect(identify('cookie', 'JSESSIONID', '')).toEqual({ provider: 'Java', weight: 30 });
		expect(identify('cookie', '_myapp_session', '')).toEqual({ provider: 'Rails', weight: 30 });
		expect(identify('cookie', 'connect.sid', '')).toEqual({ provider: 'Express', weight: 30 });
		expect(identify('cookie', '.AspNetCore.Cookies', '')).toEqual({ provider: 'ASP.NET', weight: 30 });
		expect(identify('cookie', 'ASP.NET_SessionId', '')).toEqual({ provider: 'ASP.NET', weight: 30 });
	});
});

test.describe('indexedDB rules', () => {
	test('Firebase, with and without the token manager path', () => {
		const key = 'firebaseLocalStorageDb/firebaseLocalStorage/firebase:authUser:AIzaSyABC:[DEFAULT]';
		expect(identify('indexedDb', key, 'stsTokenManager.accessToken')).toEqual({
			provider: 'Firebase',
			weight: 50
		});
		expect(identify('indexedDb', key, '')).toEqual({ provider: 'Firebase', weight: 25 });
	});
});

test.describe('ranking and misses', () => {
	test('an unrecognised key in a recognised family is not identified', () => {
		expect(identify('localStorage', 'some-random-app-token', '')).toBeNull();
	});

	test('a rule only applies to its own storage kind', () => {
		// The Auth0 SPA key is a localStorage rule; it should not leak into cookies.
		expect(identify('cookie', '@@auth0spajs@@::abc::default::openid', '')).toBeNull();
	});

	test('the same key can win a different, higher-weighted rule depending on the path', () => {
		const key = 'okta-token-storage';
		expect(identify('localStorage', key, 'accessToken.accessToken')?.weight).toBe(50);
		// A path that doesn't match the specific rule still falls back to the
		// looser one for the same key, rather than missing entirely.
		expect(identify('localStorage', key, 'idToken.idToken')).toEqual({
			provider: 'Okta',
			weight: 25
		});
	});

	test('an empty key never matches', () => {
		expect(identify('localStorage', '', '')).toBeNull();
		expect(identify('cookie', '', '')).toBeNull();
		expect(identify('indexedDb', '', '')).toBeNull();
	});
});
