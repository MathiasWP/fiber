---
'fiber': patch
---

Stop a background loader refresh from firing at a section you are signing into, and make the failure it left behind actionable.

Opening the sign-in window moves focus off the main window, and focus returning is one of the two triggers for a stale-loader refresh — so the run went out with the credential you were in the middle of replacing, failed, and posted an unattributed "the manifest request returned 403" in the sidebar at the exact moment you opened the window. Signing in never cleared it, because nothing re-ran the loader afterwards.

A section is now skipped while its sign-in window is open, the loader re-runs once a credential is captured, and the error names its section and offers a sign-in button when the API turned the credential down. Rejected manifest requests also carry a snippet of the response body, so a 403 can say which 403 it was — and a 401 or 403 in the response pane offers the same button rather than leaving you to find the drawer.
