---
"fiber": patch
---

Make the crash banner and the update toast usable while a dialog is open. An open modal dialog sets `pointer-events: none` on `<body>`, which both of them inherited: a click on Copy, Hide or Update passed straight through and landed on the dialog, which read it as an outside click and closed itself. With dialogs stacked, the banner stayed out of reach until every one of them had been dismissed. Both now take pointer events of their own and keep the click from reaching the dialog underneath.
