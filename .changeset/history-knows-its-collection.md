---
'fiber': patch
---

Keep each collection's response history to itself.

Response history was bucketed by request id alone. A loaded endpoint's id is `METHOD /path` and carries no section, so two collections describing the same API — staging and production — shared one list: opening either showed whichever had been sent last, and clearing one deleted both.

The database has stored `section_id` since the column was added; it was simply never handed back. It is now, so the window can tell the two apart, and clearing is scoped to the collection you cleared.

Entries recorded before this still show for either collection rather than disappearing, since nothing knows which one they came from. A scoped clear takes them too — they are the same request's older entries, and leaving them behind would look like the clear half-worked.
