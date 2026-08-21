---
"fiber": patch
---

An endpoint you had opened before its manifest carried a request body now picks that body up. Previously the empty one saved against it won, so the schema's body never appeared on exactly the endpoints you had used most. A body you have written is still left alone.
