---
'fiber': patch
---

Stop two loader requests cancelling each other, and say when a rejection came from somewhere else.

Every loader request for a section used the same id, and `HttpState` keys in-flight requests by that id — inserting a second under a key already there drops the first's cancel sender, which *is* the cancel signal. So a "Fetch a sample" while a background refresh was out came back "request cancelled", and which of the two died depended on timing. Loader requests now get a unique handle each; nothing cancels them by id, so there was never a reason for it to be predictable.

A rejected manifest now also reports where the response actually came from, when that left the origin the request was aimed at. A Cookie or Authorization credential is dropped on a cross-host redirect, so "403" and "403, having ended up on a different host" are different problems wearing the same status — and only one of them is your API's fault.

The sidebar's loader error is selectable and has a Copy button, because the first thing anyone does with an error they can't act on is send it to someone who can.
