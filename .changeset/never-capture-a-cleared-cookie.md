---
'fiber': patch
---

Never capture a cleared cookie, and make "Sign in again" actually sign you in.

Signing out clears a session cookie by setting it to the empty string, and some sign-in flows do it on the way through — often on the identity provider's host while the live cookie lands on another. The capture rule matched by name alone and took whichever came first, so it could store the blank one. That went out as `Cookie: sid=` and came back as the API's own version of "token is empty": a rejection that reads like a server problem and is really an empty header. A cookie now only counts if it has a value, an empty capture reports nothing rather than storing it, and the credential picker no longer offers blanks.

"Sign in again" opened Section settings on the General tab and stopped there, leaving you to find Auth and press Open sign-in yourself. It now opens the Auth tab and starts the sign-in.
