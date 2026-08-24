---
'fiber': patch
---

Stop "Pick credential…" timing out when no sign-in window is open yet.

Reading `localStorage` means evaluating script in the page, which fails until the page has loaded. That failure was propagated, discarding the cookies along with it — even though cookies are read from the Rust side, need no script, and are there immediately. So opening the picker straight from a closed window reported "timed out reading the sign-in window" while the session cookie sat in hand; opening the window first, then picking, worked.

Cookies are now kept when the page can't be read, and the timeout is only reported when there is genuinely nothing to show. The retry loop still waits for a complete read before settling, so a credential kept in `localStorage` isn't missed by returning the cookies-only snapshot the instant the window opens.
